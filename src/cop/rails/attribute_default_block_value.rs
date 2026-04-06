use crate::cop::shared::node_type::{ARRAY_NODE, CALL_NODE, HASH_NODE, KEYWORD_HASH_NODE};
use crate::cop::shared::util::{is_dsl_call, keyword_arg_value};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// FN investigation (2026-03-23): Missed `attribute :x, default: proc(&::Kind::ID)` pattern.
/// Prism stores `&arg` as a `BlockArgumentNode` in `call.block()`, not as a `BlockNode`.
/// The original check `c.block().is_none()` excluded these because `block()` was `Some`.
/// Fix: only skip when `block()` is a real `BlockNode`; flag `BlockArgumentNode` (block-pass).
pub struct AttributeDefaultBlockValue;

impl Cop for AttributeDefaultBlockValue {
    fn name(&self) -> &'static str {
        "Rails/AttributeDefaultBlockValue"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/app/models/**/*"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[ARRAY_NODE, CALL_NODE, HASH_NODE, KEYWORD_HASH_NODE]
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

        if !is_dsl_call(&call, b"attribute") {
            return;
        }

        // Check if :default keyword arg exists
        let default_value = match keyword_arg_value(&call, b"default") {
            Some(v) => v,
            None => return,
        };

        // Flag mutable/dynamic default values that should use a block:
        // Arrays, Hashes, and method calls (send nodes) are flagged.
        // String/symbol/integer/float literals and constants are accepted.
        // Calls that already have a block (e.g., `lambda { }`, `proc { }`) are
        // NOT flagged because the block already provides lazy evaluation.
        // However, calls with a block-argument (`proc(&symbol)`) ARE flagged
        // because RuboCop expects `-> { ... }` form instead. In Prism,
        // `&arg` is a BlockArgumentNode stored in `call.block()`, distinct
        // from a `BlockNode` which represents `do...end` / `{ ... }`.
        let is_mutable_call = match default_value.as_call_node() {
            Some(c) => match c.block() {
                None => true,
                Some(b) => b.as_block_argument_node().is_some(),
            },
            None => false,
        };
        let is_mutable = default_value.as_array_node().is_some()
            || default_value.as_hash_node().is_some()
            || default_value.as_keyword_hash_node().is_some()
            || is_mutable_call;

        if is_mutable {
            let loc = call.message_loc().unwrap_or(call.location());
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Pass a block to `default:` to avoid sharing mutable objects.".to_string(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        AttributeDefaultBlockValue,
        "cops/rails/attribute_default_block_value"
    );
}
