use crate::cop::shared::node_type::{CALL_NODE, SYMBOL_NODE};
use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Flags `before(:all)`, `before(:context)`, `after(:all)`, `after(:context)`.
/// These hooks can cause state to leak between tests.
///
/// ## Corpus investigation (2026-03-19)
/// FP=0, FN=12. Root cause: implementation required receiverless calls with
/// a block, but RuboCop's `def_node_matcher` uses `_` for receiver (matches
/// any, including present receivers like `config.before(:all)`) and does not
/// require a block. Calls like `@state.before(:all)`, `context.after(:context)`,
/// and `config.before :all do ... end` were all missed.
/// Fix: removed receiver and block guards, extract hook source text from byte
/// range (start of call to end of closing paren or last argument) to match
/// RuboCop's `hook.source` output.
///
/// ## Corpus investigation (2026-03-20)
/// FP=4, FN=6. Root cause: RuboCop's NodePattern `$(send _ RESTRICT_ON_SEND
/// (sym {:all :context}))` requires exactly 1 arg (the symbol) with no block
/// requirement. In Parser AST, `block_pass` (`&proc`) is a child of `send`,
/// adding to arg count, so `before(:all, &proc)` doesn't match. But calls
/// without blocks like `@state.before(:all)` DO match.
/// Previous fix over-corrected by requiring a real `BlockNode` (introduced 6 FN)
/// and not enforcing exact arg count (allowed multi-arg FP).
/// Fix: removed block requirement, added exact arg count check (`len() == 1`),
/// added block_pass exclusion (Prism stores `&arg` in `call.block()` as
/// `BlockArgumentNode`).
pub struct BeforeAfterAll;

impl Cop for BeforeAfterAll {
    fn name(&self) -> &'static str {
        "RSpec/BeforeAfterAll"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_DEFAULT_INCLUDE
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &[
            "**/spec/spec_helper.rb",
            "**/spec/rails_helper.rb",
            "**/spec/support/**/*.rb",
        ]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, SYMBOL_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_name = call.name().as_slice();
        if method_name != b"before" && method_name != b"after" {
            return;
        }

        // Check for :all or :context as first argument
        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };
        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.is_empty() {
            return;
        }

        let first_arg = &arg_list[0];
        let scope = if let Some(sym) = first_arg.as_symbol_node() {
            sym.unescaped().to_vec()
        } else {
            return;
        };

        if scope != b"all" && scope != b"context" {
            return;
        }

        // RuboCop's NodePattern requires exactly 1 arg (the symbol).
        // In Parser AST, block_pass (&proc) is a child of send, so
        // before(:all, &proc) has 2 args and doesn't match.
        // In Prism, block_pass is stored in call.block() as BlockArgumentNode,
        // not in arguments(), so we check for it separately.
        let has_block_pass = call
            .block()
            .is_some_and(|b| b.as_block_argument_node().is_some());
        if arg_list.len() != 1 || has_block_pass {
            return;
        }

        // Build hook source text matching RuboCop's `hook.source` — the send
        // node text from start of receiver (or method) through closing paren
        // or end of last argument.
        let call_start = call.location().start_offset();
        let hook_end = if let Some(close) = call.closing_loc() {
            // Parenthesized: `before(:all)` or `config.before(:all)`
            close.end_offset()
        } else {
            // No parens: `config.before :all` — end at last argument
            let last_arg = &arg_list[arg_list.len() - 1];
            last_arg.location().end_offset()
        };
        let hook = String::from_utf8_lossy(&source.as_bytes()[call_start..hook_end]);

        let (line, column) = source.offset_to_line_col(call_start);
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            format!(
                "Beware of using `{hook}` as it may cause state to leak between tests. \
                 If you are using `rspec-rails`, and `use_transactional_fixtures` is enabled, \
                 then records created in `{hook}` are not automatically rolled back."
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(BeforeAfterAll, "cops/rspec/before_after_all");
}
