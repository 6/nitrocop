use crate::cop::shared::node_type::{
    CALL_NODE, INDEX_AND_WRITE_NODE, INDEX_OPERATOR_WRITE_NODE, INDEX_OR_WRITE_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Enforces either `fetch` or `[]` for hash-style lookups.
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
                    if let Some(args) = call.arguments() {
                        let arg_list: Vec<_> = args.arguments().iter().collect();
                        if arg_list.len() == 1 && call.receiver().is_some() {
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
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(HashLookupMethod, "cops/style/hash_lookup_method");
}
