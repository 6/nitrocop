use std::str;

use regex::Regex;

use crate::cop::shared::node_type::{BLOCK_ARGUMENT_NODE, CALL_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

use super::multiline_literal_brace_layout::{self, BracePositions, METHOD_CALL_BRACE};

static LEGACY_LAYOUT_DIRECTIVE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"#\s*(?:rubocop|nitrocop)\s*:\s*(disable|enable|todo)\s+(.+)").unwrap()
});

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
///
/// Follow-up variant divergence showed two narrower gaps:
/// 1. Calls whose *only* argument is a block pass (`map(&:to_s)`,
///    `post(&builder(...))`) were skipped entirely because Prism stores that
///    `&...` in `CallNode.block()` and the regular argument list is empty.
/// 2. `same_line` false positives inside `# rubocop:disable Layout:LineLength`
///    regions came from RuboCop's legacy single-colon directive parsing:
///    `Layout:LineLength` disables the whole `Layout` department. The shared
///    directive parser does not normalize that form yet, so this cop locally
///    honors those legacy `Layout:...` disable regions until the parser is
///    fixed centrally.
pub struct MultilineMethodCallBraceLayout;

fn directive_tokens(raw: &str) -> impl Iterator<Item = &str> {
    let without_reason = match raw.find("--") {
        Some(idx) => &raw[..idx],
        None => raw,
    };

    without_reason
        .split(',')
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .map(|token| match token.find(' ') {
            Some(idx) => &token[..idx],
            None => token,
        })
        .map(|token| match token.find('(') {
            Some(idx) => &token[..idx],
            None => token,
        })
        .map(|token| {
            token.trim_end_matches(|c: char| {
                !c.is_ascii_alphanumeric() && c != '_' && c != '/' && c != ':'
            })
        })
        .filter(|token| !token.is_empty())
}

fn is_legacy_layout_single_colon(token: &str) -> bool {
    token.starts_with("Layout:") && !token.contains("::")
}

fn closes_legacy_layout_department(token: &str) -> bool {
    token == "Layout" || is_legacy_layout_single_colon(token) || token == "all"
}

fn legacy_layout_department_disabled_at_line(
    source: &SourceFile,
    parse_result: &ruby_prism::ParseResult<'_>,
    line: usize,
) -> bool {
    let lines: Vec<&[u8]> = source.lines().collect();
    let mut layout_disabled = false;

    for comment in parse_result.comments() {
        let loc = comment.location();
        let (comment_line, col) = source.offset_to_line_col(loc.start_offset());

        if comment_line > line {
            return layout_disabled;
        }

        let comment_bytes = &source.as_bytes()[loc.start_offset()..loc.end_offset()];
        let Ok(comment_str) = str::from_utf8(comment_bytes) else {
            continue;
        };

        let Some(caps) = LEGACY_LAYOUT_DIRECTIVE_RE.captures(comment_str) else {
            continue;
        };

        let is_inline = if comment_line >= 1 && comment_line <= lines.len() {
            let line_bytes = lines[comment_line - 1];
            let before_comment = &line_bytes[..col.min(line_bytes.len())];
            before_comment.iter().any(|b| !b.is_ascii_whitespace())
        } else {
            false
        };

        // Match the shared directive parser's YARD/doc-example guard so we only
        // honor real directives.
        let match_start = caps.get(0).unwrap().start();
        if match_start > 0 && !is_inline {
            let prefix = &comment_str[..match_start];
            if prefix.bytes().all(|b| b == b'#' || b == b' ' || b == b'\t') {
                continue;
            }
        }

        let action = caps.get(1).unwrap().as_str();
        let tokens: Vec<&str> = directive_tokens(caps.get(2).unwrap().as_str()).collect();

        match action {
            "disable" | "todo" => {
                if tokens
                    .iter()
                    .any(|token| is_legacy_layout_single_colon(token))
                {
                    if is_inline && comment_line == line {
                        return true;
                    }
                    if !is_inline {
                        layout_disabled = true;
                    }
                }
            }
            "enable" if !is_inline => {
                if tokens
                    .iter()
                    .any(|token| closes_legacy_layout_department(token))
                {
                    layout_disabled = false;
                }
            }
            _ => {}
        }
    }

    layout_disabled
}

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
        parse_result: &ruby_prism::ParseResult<'_>,
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

        let arg_list: Vec<ruby_prism::Node<'_>> = call
            .arguments()
            .map(|args| args.arguments().iter().collect())
            .unwrap_or_default();
        let block_arg = call
            .block()
            .and_then(|block| block.as_block_argument_node().map(|_| block));

        if arg_list.is_empty() && block_arg.is_none() {
            return;
        }

        let last_arg = block_arg
            .as_ref()
            .unwrap_or_else(|| arg_list.last().unwrap());
        if multiline_literal_brace_layout::last_line_heredoc(source, last_arg) {
            return;
        }

        let (open_line, _) = source.offset_to_line_col(opening.start_offset());
        let (close_line, close_col) = source.offset_to_line_col(closing.start_offset());

        let first_arg = arg_list.first().unwrap_or(last_arg);
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
        let last_arg_end = if let Some(block) = block_arg.as_ref() {
            if let Some(expr) = block.as_block_argument_node().and_then(|b| b.expression()) {
                expr.location().end_offset().saturating_sub(1)
            } else {
                block.location().end_offset().saturating_sub(1)
            }
        } else {
            last_arg.location().end_offset().saturating_sub(1)
        };
        let (last_arg_line, _) = source.offset_to_line_col(last_arg_end);

        let mut local_diagnostics = Vec::new();
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
            &mut local_diagnostics,
        );

        diagnostics.extend(local_diagnostics.into_iter().filter(|diagnostic| {
            !legacy_layout_department_disabled_at_line(
                source,
                parse_result,
                diagnostic.location.line,
            )
        }));
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

    fn style_config(style: &str) -> CopConfig {
        let mut opts = std::collections::HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String(style.to_string()),
        );
        CopConfig {
            options: opts,
            ..CopConfig::default()
        }
    }

    #[test]
    fn same_line_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &MultilineMethodCallBraceLayout,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/multiline_method_call_brace_layout/same_line_offense.rb"
            ),
            style_config("same_line"),
        );
    }

    #[test]
    fn same_line_no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &MultilineMethodCallBraceLayout,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/multiline_method_call_brace_layout/same_line_no_offense.rb"
            ),
            style_config("same_line"),
        );
    }

    #[test]
    fn new_line_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &MultilineMethodCallBraceLayout,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/multiline_method_call_brace_layout/new_line_offense.rb"
            ),
            style_config("new_line"),
        );
    }

    #[test]
    fn new_line_no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &MultilineMethodCallBraceLayout,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/multiline_method_call_brace_layout/new_line_no_offense.rb"
            ),
            style_config("new_line"),
        );
    }

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
