use crate::cop::shared::node_type::{BLOCK_ARGUMENT_NODE, CALL_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

use super::multiline_literal_brace_layout::{self, BracePositions, METHOD_CALL_BRACE};

/// ## Corpus investigation (2026-03-10)
///
/// Corpus oracle reported FP=0, FN=3.
///
/// FP=0: previous false positives in heredoc-heavy calls were fixed by
/// recursing into nested call arguments, keyword hashes, and assoc values when
/// checking whether the last argument contains a conflicting heredoc.
///
/// FN=3: this cop previously skipped brace-layout checks when *any* argument
/// contained a heredoc. RuboCop only skips when the *last* argument contains a
/// heredoc terminator that forces the closing parenthesis placement. Narrowing
/// the skip to the last argument fixes heredoc-first calls like
/// `foo(<<~EOS, arg ... ).call`.
///
/// ## Corpus investigation (2026-03-29)
///
/// FN=2: outer calls like `wrapper(Hash.from_xml(<<-XML ... XML ))` were still
/// skipped because the last argument contained a nested heredoc somewhere in
/// its subtree. RuboCop only skips when that descendant heredoc reaches the
/// last line of the last-argument node itself. Nested calls whose own closing
/// `)` lands after the heredoc terminator must still be checked.
///
/// ## Variant style fix (2026-04)
///
/// For `same_line` and `new_line` styles, `BlockArgumentNode` (e.g., `&:to_s`,
/// `&method(:foo)`) was causing FNs because `node_last_line` returned the line
/// of the closing `)` instead of the actual content. RuboCop uses
/// `children(node).last.last_line` for this case; Prism's
/// `BlockArgumentNode.expression` provides the equivalent inner content.
pub struct MultilineMethodCallBraceLayout;

impl Cop for MultilineMethodCallBraceLayout {
    fn name(&self) -> &'static str {
        "Layout/MultilineMethodCallBraceLayout"
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
        let enforced_style = config.get_str("EnforcedStyle", "symmetrical");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // Must have explicit parentheses
        let opening = match call.opening_loc() {
            Some(loc) => loc,
            None => return,
        };
        let closing = match call.closing_loc() {
            Some(loc) => loc,
            None => return,
        };

        if opening.as_slice() != b"(" || closing.as_slice() != b")" {
            return;
        }

        // Must have arguments
        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };

        let arg_list: Vec<ruby_prism::Node<'_>> = args.arguments().iter().collect();
        if arg_list.is_empty() {
            return;
        }

        let last_arg = arg_list.last().unwrap();
        if multiline_literal_brace_layout::last_line_heredoc(source, last_arg) {
            return;
        }

        let (open_line, _) = source.offset_to_line_col(opening.start_offset());
        let (close_line, close_col) = source.offset_to_line_col(closing.start_offset());

        let first_arg = &arg_list[0];
        let (first_arg_line, _) = source.offset_to_line_col(first_arg.location().start_offset());

        // Compute the effective end of the last argument. In Prism, `&block`
        // arguments are stored in the CallNode's `block` field, not in the
        // arguments list. For `define_method(method, &lambda do...end)`, the
        // BlockArgumentNode's end offset includes the block's `end`, so use
        // it when present to correctly determine the last arg's line.
        //
        // For BlockArgumentNode (e.g., `&:to_s`), the node's location().end_offset
        // points to the closing `)`, but we need the last line of the actual content
        // (e.g., the `:to_s` symbol). RuboCop uses `children(node).last.last_line`
        // for this case. In Prism, `BlockArgumentNode.expression` holds the inner
        // content, so use its end offset.
        let last_arg_end = if let Some(block) = call.block() {
            if block.as_block_argument_node().is_some() {
                // &block_arg — use expression end to get actual content end
                if let Some(expr) = block.as_block_argument_node().and_then(|b| b.expression()) {
                    expr.location().end_offset().saturating_sub(1)
                } else {
                    block.location().end_offset().saturating_sub(1)
                }
            } else {
                // Regular do...end block — `)` comes before the block, not after
                last_arg.location().end_offset().saturating_sub(1)
            }
        } else {
            last_arg.location().end_offset().saturating_sub(1)
        };
        let (last_arg_line, _) = source.offset_to_line_col(last_arg_end);

        multiline_literal_brace_layout::check_brace_layout(
            self,
            source,
            enforced_style,
            &METHOD_CALL_BRACE,
            &BracePositions {
                open_line,
                close_line,
                close_col,
                first_elem_line: first_arg_line,
                last_elem_line: last_arg_line,
            },
            diagnostics,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        MultilineMethodCallBraceLayout,
        "cops/layout/multiline_method_call_brace_layout"
    );

    #[test]
    fn heredoc_only_in_earlier_argument_still_checks_brace_layout() {
        let source = br#"foo(<<~EOS, arg
  text
EOS
).do_something
"#;
        let diagnostics = run_cop_full(&MultilineMethodCallBraceLayout, source);
        assert_eq!(
            diagnostics.len(),
            1,
            "Expected one offense: {diagnostics:?}"
        );
    }
}
