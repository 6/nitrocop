use crate::cop::shared::node_type::{
    CALL_NODE, INDEX_AND_WRITE_NODE, INDEX_OPERATOR_WRITE_NODE, INDEX_OR_WRITE_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Enforces either `fetch` or `[]` for hash-style lookups.
///
/// ## Corpus investigation (2026-03-30)
///
/// Prism stores `&block` as `call.block()` with a `BlockArgumentNode`, while
/// RuboCop's Parser-backed `node.arguments.one?` counts that block-pass as an
/// argument. The original implementation only looked at `call.arguments()` and
/// rejected every call with `call.block()`, so it missed `receiver.fetch(&block)`.
///
/// Fix: count `BlockArgumentNode` in the effective argument count for
/// `EnforcedStyle: brackets`, but continue excluding literal blocks (`{}` /
/// `do...end`) so `fetch(key) { default }` remains allowed.
///
/// ## Variant style divergence (2026-04-06)
///
/// The default `EnforcedStyle: brackets` is correct.
///
/// `EnforcedStyle: fetch` had FN regression because Prism represents compound
/// index assignments (`hash[key] ||= val`) as `IndexOrWriteNode` /
/// `IndexAndWriteNode` / `IndexOperatorWriteNode` instead of nested `CallNode`s.
/// RuboCop's Parser represents these as `or_asgn(send(... :[] ...), ...)` where
/// the inner `send` with `[]` IS visited and flagged.
///
/// Fix: added the index-write node types to `interested_node_types` and handle
/// them in the `fetch` style branch by treating them as implicit `[]` reads.
///
/// ## Variant style divergence (2026-04-07) - fetch style FP/FN
///
/// `EnforcedStyle: fetch` had two bugs related to block argument handling:
///
/// 1. FN: `hash[&block]` and `hash[&-> {...}]` were not flagged because
///    `call.arguments()` returns `None` in Prism when only a block argument
///    is present. The `if let Some(args) = call.arguments()` pattern skipped
///    entirely in this case. RuboCop's `node.arguments.one?` counts block_pass
///    as an argument, so these should be flagged.
///
/// 2. FP: `hash[key, &block]` was incorrectly flagged because `call.arguments()`
///    doesn't include `BlockArgumentNode` - it only counts regular arguments.
///    So `arg_list.len() == 1` was true (only `key`), but RuboCop counts
///    both `key` and `&block` as arguments, so `arguments.one?` is false.
///
/// Fix: Use `.map_or(0, ...)` to handle `None` from `call.arguments()`, and
/// add `call.block_argument_node().is_some()` to the effective argument count
/// so block-only and block-plus-arg cases are correctly distinguished.
pub struct HashLookupMethod;

impl Cop for HashLookupMethod {
    fn name(&self) -> &'static str {
        "Style/HashLookupMethod"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CALL_NODE,
            INDEX_AND_WRITE_NODE,
            INDEX_OPERATOR_WRITE_NODE,
            INDEX_OR_WRITE_NODE,
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
        let style = config.get_str("EnforcedStyle", "brackets");

        // Handle IndexOrWriteNode, IndexAndWriteNode, IndexOperatorWriteNode
        // for EnforcedStyle: fetch
        if style == "fetch" {
            // All three index-write node types have the same structure:
            // receiver (the hash), arguments (the key), and a block/operator part.
            // We flag them the same way as implicit [] reads.
            if let Some(write) = node.as_index_or_write_node() {
                if let Some(args) = write.arguments() {
                    let arg_list: Vec<_> = args.arguments().iter().collect();
                    if arg_list.len() == 1 && write.receiver().is_some() {
                        let loc = write.location();
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Use `fetch` instead of `[]`.".to_string(),
                        ));
                    }
                }
                return;
            }
            if let Some(write) = node.as_index_and_write_node() {
                if let Some(args) = write.arguments() {
                    let arg_list: Vec<_> = args.arguments().iter().collect();
                    if arg_list.len() == 1 && write.receiver().is_some() {
                        let loc = write.location();
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Use `fetch` instead of `[]`.".to_string(),
                        ));
                    }
                }
                return;
            }
            if let Some(write) = node.as_index_operator_write_node() {
                if let Some(args) = write.arguments() {
                    let arg_list: Vec<_> = args.arguments().iter().collect();
                    if arg_list.len() == 1 && write.receiver().is_some() {
                        let loc = write.location();
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Use `fetch` instead of `[]`.".to_string(),
                        ));
                    }
                }
                return;
            }
        }

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_bytes = call.name().as_slice();

        match style {
            "brackets" => {
                // Flag fetch calls, suggest []
                if method_bytes == b"fetch" {
                    let has_block_arg = call
                        .block()
                        .is_some_and(|block| block.as_block_argument_node().is_some());
                    let has_block_literal = call
                        .block()
                        .is_some_and(|block| block.as_block_node().is_some());
                    let effective_arg_count = call
                        .arguments()
                        .map_or(0, |args| args.arguments().iter().count())
                        + usize::from(has_block_arg);

                    // RuboCop counts `&block` toward `arguments.one?`, but still
                    // allows literal blocks (`fetch(key) { ... }`).
                    if effective_arg_count == 1 && !has_block_literal && call.receiver().is_some() {
                        let loc = call.message_loc().unwrap_or_else(|| call.location());
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Use `[]` instead of `fetch`.".to_string(),
                        ));
                    }
                }
            }
            "fetch" => {
                // Flag [] calls, suggest fetch
                if method_bytes == b"[]" {
                    // RuboCop's `arguments.one?` counts block_pass as an argument.
                    // Prism's `call.arguments()` does NOT include block_pass,
                    // and may return None entirely when only a block arg is present.
                    // Use map_or to safely handle None (count = 0).
                    let arg_count = call
                        .arguments()
                        .map_or(0, |args| args.arguments().iter().count());
                    let has_block_arg = call
                        .block()
                        .is_some_and(|b| b.as_block_argument_node().is_some());
                    let effective_arg_count = arg_count + usize::from(has_block_arg);
                    if effective_arg_count == 1 && call.receiver().is_some() {
                        let loc = call.location();
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Use `fetch` instead of `[]`.".to_string(),
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(HashLookupMethod, "cops/style/hash_lookup_method");

    fn fetch_style_config() -> CopConfig {
        let mut opts = std::collections::HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("fetch".to_string()),
        );
        CopConfig {
            options: opts,
            ..CopConfig::default()
        }
    }

    #[test]
    fn fetch_style_offense() {
        // FN cases: block-only arguments should be flagged
        crate::testutil::assert_cop_offenses_full_with_config(
            &HashLookupMethod,
            include_bytes!(
                "../../../tests/fixtures/cops/style/hash_lookup_method/fetch_style_offense.rb"
            ),
            fetch_style_config(),
        );
    }

    #[test]
    fn fetch_style_no_offense() {
        // Cases with multiple args should not be flagged
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HashLookupMethod,
            include_bytes!(
                "../../../tests/fixtures/cops/style/hash_lookup_method/fetch_style_no_offense.rb"
            ),
            fetch_style_config(),
        );
    }
}
