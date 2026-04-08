use crate::cop::shared::node_type::{BLOCK_ARGUMENT_NODE, CALL_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Enforces a consistent choice between `then` and `yield_self`.
///
/// ## Variant style divergence (2026-04-08)
///
/// `EnforcedStyle: yield_self` had 5 false positives in the corpus, all for
/// `then` calls that passed extra positional arguments alongside a block pass,
/// such as `future.then(*args, &task)`.
///
/// RuboCop only inspects block-pass sends via `on_send` when the send has
/// exactly one argument and that argument is the block pass. Prism stores
/// `&block` in `call.block()` as a `BlockArgumentNode`, not in
/// `call.arguments()`, so treating every `call.block().is_some()` as equivalent
/// to RuboCop's `on_send` over-reported these multi-argument calls.
///
/// Fix: keep flagging literal block forms (`then { ... }`) regardless of
/// argument count, but only flag block-pass sends when the block pass is the
/// sole effective argument.
pub struct ObjectThen;

impl Cop for ObjectThen {
    fn name(&self) -> &'static str {
        "Style/ObjectThen"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[BLOCK_ARGUMENT_NODE, CALL_NODE]
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
        let enforced_style = config.get_str("EnforcedStyle", "then");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_name = call.name();
        let method_bytes = method_name.as_slice();

        // Check if this is yield_self or then
        if !matches!(method_bytes, b"yield_self" | b"then") {
            return;
        }

        let has_block_pass = call
            .block()
            .is_some_and(|block| block.as_block_argument_node().is_some());
        let has_literal_block = call.block().is_some() && !has_block_pass;

        // RuboCop's on_block path flags literal blocks regardless of argument
        // count, but its on_send path only flags block-pass sends when the
        // block pass is the sole effective argument.
        let has_supported_invocation = has_literal_block
            || (has_block_pass
                && call
                    .arguments()
                    .is_none_or(|args| args.arguments().is_empty()));

        if !has_supported_invocation {
            return;
        }

        if enforced_style == "then" && method_bytes == b"yield_self" {
            let msg_loc = match call.message_loc() {
                Some(l) => l,
                None => return,
            };
            let (line, column) = source.offset_to_line_col(msg_loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Prefer `then` over `yield_self`.".to_string(),
            ));
        } else if enforced_style == "yield_self" && method_bytes == b"then" {
            let msg_loc = match call.message_loc() {
                Some(l) => l,
                None => return,
            };
            let (line, column) = source.offset_to_line_col(msg_loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Prefer `yield_self` over `then`.".to_string(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ObjectThen, "cops/style/object_then");

    fn yield_self_config() -> CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("yield_self".to_string()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    #[test]
    fn yield_self_style_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ObjectThen,
            include_bytes!("../../../tests/fixtures/cops/style/object_then/yield_self_offense.rb"),
            yield_self_config(),
        );
    }

    #[test]
    fn yield_self_style_no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ObjectThen,
            include_bytes!(
                "../../../tests/fixtures/cops/style/object_then/yield_self_no_offense.rb"
            ),
            yield_self_config(),
        );
    }
}
