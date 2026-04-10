use crate::cop::shared::node_type::{
    BEGIN_NODE, CALL_NODE, CASE_NODE, ELSE_NODE, ENSURE_NODE, FOR_NODE, IF_NODE, IN_NODE,
    PROGRAM_NODE, RESCUE_NODE, STATEMENTS_NODE, UNLESS_NODE, UNTIL_NODE, WHEN_NODE, WHILE_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Checks for consecutive loops over the same data that can be combined.
///
/// ## Investigation Notes
///
/// FP root cause: nitrocop included non-looping methods (map, flat_map, select,
/// reject, collect) in the loop method list. RuboCop only considers methods
/// starting with "each" or ending with "_each". Also, the blank-line gap check
/// was wrong — RuboCop doesn't care about blank lines between consecutive loops,
/// only about intervening *statements*. The `left_sibling` in RuboCop is the
/// previous AST sibling, regardless of whitespace.
///
/// Additional FP root cause: calls with block arguments (`each(&:foo)`) are NOT
/// block nodes in RuboCop's AST, so `on_block` never fires for them. nitrocop
/// was treating `BlockArgumentNode` the same as `BlockNode`, causing false
/// positives when consecutive `each(&:symbol)` calls appeared.
///
/// FN root cause: `for` loops were not handled at all (only CallNode was checked).
/// Methods like `each_key`, `each_value`, `each_pair`, `each_with_object` were
/// missing from the method list because it was a hardcoded allowlist instead of
/// using the `starts_with("each") || ends_with("_each")` pattern from RuboCop.
/// Also, RuboCop requires both loops to have bodies (not empty blocks).
///
/// Additional FN root cause: receiverless loop calls (implicit self, e.g. bare
/// `each do |item| ... end`) were not handled because `call.receiver()` returning
/// `None` caused `get_loop_info` to return `None`.
///
/// Additional FP/FN root causes fixed here:
/// - Pure explicit `begin .. end` (`kwbegin`) bodies are not scanned by RuboCop,
///   but handled `begin` bodies with `rescue`/`else`/`ensure` are. Prism uses
///   `BeginNode` for both, so this cop now only scans `BeginNode` statements
///   when one of those handler clauses is present.
/// - Prism's `CallNode#location` for heredoc receivers omits the heredoc body
///   (`<<END.split`), so raw source slicing made different heredocs look equal.
///   The receiver/argument comparison now uses a structural key for nested calls.
/// - Prism's visitor can bypass `StatementsNode` dispatch for some containers,
///   and `for` / `case ... else` bodies need explicit extraction so nested
///   consecutive loops are still compared.
/// - `for` loop collection comparison used raw source text, so `[[1,2]]` and
///   `[[1, 2]]` were treated as different collections. RuboCop uses AST equality.
///   Fixed by building structural keys for `ArrayNode` from element keys,
///   normalizing away whitespace differences.
pub struct CombinableLoops;

impl Cop for CombinableLoops {
    fn name(&self) -> &'static str {
        "Style/CombinableLoops"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            BEGIN_NODE,
            CALL_NODE,
            CASE_NODE,
            FOR_NODE,
            PROGRAM_NODE,
            STATEMENTS_NODE,
            // Container nodes whose StatementsNode children bypass
            // visit_branch_node_enter in Prism's visitor:
            ELSE_NODE,
            ENSURE_NODE,
            IF_NODE,
            IN_NODE,
            UNLESS_NODE,
            UNTIL_NODE,
            WHEN_NODE,
            WHILE_NODE,
            RESCUE_NODE,
        ]
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
        let stmt_list: Vec<ruby_prism::Node<'_>> =
            if let Some(stmts_node) = node.as_statements_node() {
                stmts_node.body().iter().collect()
            } else if let Some(prog_node) = node.as_program_node() {
                prog_node.statements().body().iter().collect()
            } else if let Some(stmts) = extract_statements(node) {
                stmts.body().iter().collect()
            } else {
                return;
            };

        for i in 1..stmt_list.len() {
            let prev = &stmt_list[i - 1];
            let curr = &stmt_list[i];

            if let (Some(prev_info), Some(curr_info)) =
                (get_loop_info(source, prev), get_loop_info(source, curr))
            {
                if prev_info.receiver == curr_info.receiver
                    && prev_info.method == curr_info.method
                    && prev_info.arguments == curr_info.arguments
                {
                    let loc = curr.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Combine this loop with the previous loop.".to_string(),
                    ));
                }
            }
        }
    }
}

/// Extract the `StatementsNode` from container nodes that bypass
/// `visit_branch_node_enter` in Prism's visitor.
fn extract_statements<'pr>(
    node: &ruby_prism::Node<'pr>,
) -> Option<ruby_prism::StatementsNode<'pr>> {
    if let Some(n) = node.as_if_node() {
        return n.statements();
    }
    if let Some(n) = node.as_begin_node() {
        if n.rescue_clause().is_some() || n.else_clause().is_some() || n.ensure_clause().is_some() {
            return n.statements();
        }
        return None;
    }
    if let Some(n) = node.as_unless_node() {
        return n.statements();
    }
    if let Some(n) = node.as_else_node() {
        return n.statements();
    }
    if let Some(n) = node.as_when_node() {
        return n.statements();
    }
    if let Some(n) = node.as_while_node() {
        return n.statements();
    }
    if let Some(n) = node.as_until_node() {
        return n.statements();
    }
    if let Some(n) = node.as_ensure_node() {
        return n.statements();
    }
    if let Some(n) = node.as_in_node() {
        return n.statements();
    }
    if let Some(n) = node.as_rescue_node() {
        return n.statements();
    }
    if let Some(n) = node.as_for_node() {
        return n.statements();
    }
    if let Some(n) = node.as_case_node() {
        return n
            .else_clause()
            .and_then(|else_clause| else_clause.statements());
    }
    None
}

struct LoopInfo {
    receiver: String,
    method: String,
    arguments: String,
}

fn is_collection_looping_method(method_name: &str) -> bool {
    method_name.starts_with("each") || method_name.ends_with("_each")
}

fn get_loop_info(source: &SourceFile, node: &ruby_prism::Node<'_>) -> Option<LoopInfo> {
    // Handle for loops
    if let Some(for_node) = node.as_for_node() {
        return Some(LoopInfo {
            receiver: node_key(source, &for_node.collection())?,
            method: "for".to_string(),
            arguments: String::new(),
        });
    }

    // Handle method call loops (each, each_with_index, etc.)
    let call = node.as_call_node()?;
    let method_name = std::str::from_utf8(call.name().as_slice()).ok()?;

    if !is_collection_looping_method(method_name) {
        return None;
    }

    // Must have a real block (not a block argument like &:foo)
    let block = call.block()?;
    let block_node = block.as_block_node()?;

    // Both loops must have bodies (not empty blocks)
    block_node.body()?;

    // Handle receiverless calls (implicit self)
    let receiver_text = if let Some(receiver) = call.receiver() {
        node_key(source, &receiver)?
    } else {
        String::new()
    };

    // Capture method arguments (e.g., each_with_object([]) — the `([])` part)
    let arguments_text = if let Some(args) = call.arguments() {
        arguments_key(source, &args)?
    } else {
        String::new()
    };

    Some(LoopInfo {
        receiver: receiver_text,
        method: method_name.to_string(),
        arguments: arguments_text,
    })
}

fn node_key(source: &SourceFile, node: &ruby_prism::Node<'_>) -> Option<String> {
    if let Some(call) = node.as_call_node() {
        let receiver = if let Some(receiver) = call.receiver() {
            node_key(source, &receiver)?
        } else {
            String::new()
        };
        let operator = call
            .call_operator_loc()
            .and_then(|loc| source.try_byte_slice(loc.start_offset(), loc.end_offset()))
            .unwrap_or(".");
        let method_name = std::str::from_utf8(call.name().as_slice()).ok()?;
        let arguments = if let Some(args) = call.arguments() {
            arguments_key(source, &args)?
        } else {
            String::new()
        };
        let block = if let Some(block) = call.block() {
            format!("{{{}}}", node_key(source, &block)?)
        } else {
            String::new()
        };

        return Some(format!(
            "{receiver}{operator}{method_name}{arguments}{block}"
        ));
    }

    // Build structural keys for array literals so that `[[1,2]]` and `[[1, 2]]` match
    if let Some(array) = node.as_array_node() {
        let mut parts = Vec::new();
        for element in array.elements().iter() {
            parts.push(node_key(source, &element)?);
        }
        return Some(format!("[{}]", parts.join(",")));
    }

    if let Some(string) = node.as_string_node() {
        return string_node_key(source, &string);
    }

    if let Some(string) = node.as_interpolated_string_node() {
        return interpolated_string_node_key(source, &string);
    }

    // Symbols: include ":" prefix to distinguish from bare strings (e.g. %i vs %w elements)
    if let Some(sym) = node.as_symbol_node() {
        let name = std::str::from_utf8(sym.unescaped()).ok()?;
        return Some(format!(":{name}"));
    }

    // Boolean/nil literals: prefix to distinguish from %w string elements with same text
    if node.as_true_node().is_some() {
        return Some("kw:true".to_string());
    }
    if node.as_false_node().is_some() {
        return Some("kw:false".to_string());
    }
    if node.as_nil_node().is_some() {
        return Some("kw:nil".to_string());
    }

    source
        .try_byte_slice(node.location().start_offset(), node.location().end_offset())
        .map(ToOwned::to_owned)
}

fn arguments_key(source: &SourceFile, args: &ruby_prism::ArgumentsNode<'_>) -> Option<String> {
    let mut parts = Vec::new();
    for argument in args.arguments().iter() {
        parts.push(node_key(source, &argument)?);
    }
    if parts.is_empty() {
        Some(String::new())
    } else {
        Some(format!("[{}]", parts.join(",")))
    }
}

fn string_node_key(source: &SourceFile, node: &ruby_prism::StringNode<'_>) -> Option<String> {
    if let Some(opening) = node.opening_loc() {
        let opening_text = slice_text(source, opening)?;
        if opening_text.starts_with("<<") {
            let mut key = opening_text;
            let content = slice_text(source, node.content_loc())?;
            key.push_str(&content);
            if let Some(closing) = node.closing_loc() {
                let closing = slice_text(source, closing)?;
                key.push_str(&closing);
            }
            return Some(key);
        }
    }

    source
        .try_byte_slice(node.location().start_offset(), node.location().end_offset())
        .map(ToOwned::to_owned)
}

fn interpolated_string_node_key(
    source: &SourceFile,
    node: &ruby_prism::InterpolatedStringNode<'_>,
) -> Option<String> {
    if let Some(opening) = node.opening_loc() {
        let opening_text = slice_text(source, opening)?;
        if opening_text.starts_with("<<") {
            let mut key = opening_text;
            for part in node.parts().iter() {
                key.push_str(&node_key(source, &part)?);
            }
            if let Some(closing) = node.closing_loc() {
                let closing = slice_text(source, closing)?;
                key.push_str(&closing);
            }
            return Some(key);
        }
    }

    source
        .try_byte_slice(node.location().start_offset(), node.location().end_offset())
        .map(ToOwned::to_owned)
}

fn slice_text(source: &SourceFile, loc: ruby_prism::Location<'_>) -> Option<String> {
    source
        .try_byte_slice(loc.start_offset(), loc.end_offset())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(CombinableLoops, "cops/style/combinable_loops");
}
