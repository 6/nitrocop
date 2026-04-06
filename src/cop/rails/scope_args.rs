use crate::cop::shared::node_type::{CALL_NODE, LAMBDA_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

pub struct ScopeArgs;

impl Cop for ScopeArgs {
    fn name(&self) -> &'static str {
        "Rails/ScopeArgs"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/app/models/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, LAMBDA_NODE]
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
        if call.receiver().is_some() {
            return;
        }
        if call.name().as_slice() != b"scope" {
            return;
        }
        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };
        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.len() < 2 {
            return;
        }
        let second = &arg_list[1];

        // RuboCop pattern: (send nil? :scope _ $send)
        // Only flag when the second argument is a plain method call (send node)
        // without an attached block. This matches model scope usage like:
        //   scope :something, where(something: true)
        let second_call = match second.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // If the call has a block attached, it's things like `Proc.new { }` or `proc { }` - not an offense
        if second_call.block().is_some() {
            return;
        }

        // Lambda literal is fine
        if second.as_lambda_node().is_some() {
            return;
        }

        // proc/lambda calls without blocks are already method calls, but
        // `proc` and `lambda` called without a block is unusual; skip them
        let name = second_call.name().as_slice();
        if name == b"proc" || name == b"lambda" {
            return;
        }

        let loc = second.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            "Use `lambda`/`proc` instead of a plain method call.".to_string(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ScopeArgs, "cops/rails/scope_args");
}
