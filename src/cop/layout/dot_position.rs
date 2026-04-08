use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-09)
///
/// Corpus oracle reported FP=2, FN=0.
///
/// FP=2: Fixed by skipping `::` scope resolution operators — only `.` and `&.` should be checked.
/// The 2 FPs were from rufo's spec file with `foo::\n bar` patterns.
///
/// ## Variant style divergence (trailing, 2026-04-06)
///
/// With `EnforcedStyle: trailing`, nitrocop misses offenses where the receiver is
/// a heredoc. RuboCop correctly detects these because its `receiver_end_line()`
/// uses `last_heredoc_line()` to find the line after the heredoc body, not the
/// receiver node's raw end offset.
///
/// The fix: when the receiver is a heredoc (any string type with heredoc flag),
/// use the heredoc end line as the receiver end line for blank-line distance checks.
///
/// ## Variant FN fix: blank-line skip logic + .() calls (2026-04-08)
///
/// Two FN sources with `EnforcedStyle: trailing`:
///
/// 1. **Blank-line skip logic**: nitrocop checked `(dot_line - recv_end_line).abs() > 1`
///    separately, which skips cases where a comment or gap exists between receiver
///    and dot—even when dot and selector are on the same line. RuboCop uses
///    `(selector_line - max(receiver_line, dot_line)) > 1`, which only skips when
///    the selector is far from both the receiver and dot. Fixed to match RuboCop.
///
/// 2. **`.()` call syntax**: nitrocop returned early when `message_loc()` was None,
///    missing `.()` calls (implicit `call` method). RuboCop falls back to
///    `node.loc.begin` (opening paren) as the selector. Fixed to use `opening_loc()`
///    when `message_loc()` is absent.
///
/// ## Variant FP fix: heredoc same-line check (2026-04-08)
///
/// The blank-line skip fix introduced FPs in `trailing` mode for heredoc receivers
/// with inline method calls (e.g. `<<~SQL.squish`). In Parser gem (RuboCop), a
/// heredoc node's `source_range` covers only the opening tag (`<<~SQL`), so
/// `same_line?(selector_range, end_range(receiver))` returns true for these calls.
/// In Prism, the node location spans the full heredoc body including the closing
/// delimiter, making the selector appear on a different line than the receiver end.
///
/// Fixed by adding a same-line check using the receiver's start line: if the
/// selector is on the same line as the receiver start, the call is always
/// single-line and no offense should be reported.
///
/// Remaining FP (12): caused by `# rubocop:disable Layout:LineLength` using colon
/// syntax, which RuboCop interprets as a department-level disable (all Layout cops).
/// This is a disable-comment handling issue, not a DotPosition detection bug.
pub struct DotPosition;

impl Cop for DotPosition {
    fn name(&self) -> &'static str {
        "Layout/DotPosition"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
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
        let style = config.get_str("EnforcedStyle", "leading");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // Must have a dot (regular `.` or safe navigation `&.`)
        let dot_loc = match call.call_operator_loc() {
            Some(loc) => loc,
            None => return,
        };

        // Skip `::` scope resolution operators — only `.` and `&.` are relevant
        if dot_loc.as_slice() == b"::" {
            return;
        }

        // Must have a receiver
        let receiver = match call.receiver() {
            Some(r) => r,
            None => return,
        };

        // Must have a method name (message) or an opening paren for `.()` calls.
        // RuboCop uses `node.loc.selector || node.loc.begin` as the selector range.
        let msg_loc = match call.message_loc().or_else(|| call.opening_loc()) {
            Some(loc) => loc,
            None => return,
        };

        let (dot_line, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
        let recv_end_offset = receiver.location().end_offset().saturating_sub(1);
        let recv_end_line = if is_heredoc_node(&receiver) {
            // For heredocs, the receiver location end is INSIDE the heredoc body,
            // but the heredoc ends AFTER the closing delimiter.
            // Use the heredoc end line for accurate distance checks.
            receiver_heredoc_end_line(source, &receiver)
                .unwrap_or_else(|| source.offset_to_line_col(recv_end_offset).0)
        } else if let Some(_call) = receiver.as_call_node() {
            // For call chains whose receiver is ALSO a call, we may need to trace
            // through multiple layers to find a nested heredoc receiver.
            call_chain_heredoc_end_line(source, &receiver)
                .unwrap_or_else(|| source.offset_to_line_col(recv_end_offset).0)
        } else {
            source.offset_to_line_col(recv_end_offset).0
        };
        let (msg_line, _) = source.offset_to_line_col(msg_loc.start_offset());

        // RuboCop: same_line?(selector_range, end_range(node.receiver))
        // In Parser gem, heredoc source_range covers only the opening tag,
        // so `<<~SQL.squish` has receiver end on the opening line. In Prism,
        // node location spans the full heredoc body. Using receiver start line
        // as a safe proxy: if selector is on the same line, the call is single-line.
        let recv_start_line = source
            .offset_to_line_col(receiver.location().start_offset())
            .0;
        if msg_line == recv_start_line {
            return;
        }

        // Single line call — no issue
        if msg_line == recv_end_line {
            return;
        }

        // Skip if there's a blank line between the selector and the highest of
        // (receiver end, dot position). Matches RuboCop's `line_between?` check:
        //   return true if line_between?(selector_line, [receiver_line, dot_line].max)
        // where line_between?(a, b) = (a - b) > 1
        let max_line = recv_end_line.max(dot_line);
        if (msg_line as i64 - max_line as i64) > 1 {
            return;
        }

        let dot_str = std::str::from_utf8(dot_loc.as_slice()).unwrap_or(".");

        match style {
            "trailing" => {
                // Dot should be on the same line as the receiver (trailing)
                if dot_line != recv_end_line {
                    diagnostics.push(self.diagnostic(
                        source,
                        dot_line,
                        dot_col,
                        format!(
                            "Place the `{}` on the previous line, together with the method call receiver.",
                            dot_str
                        ),
                    ));
                }
            }
            _ => {
                // "leading" (default): dot should be on the same line as the method name
                if dot_line != msg_line {
                    diagnostics.push(self.diagnostic(
                        source,
                        dot_line,
                        dot_col,
                        format!(
                            "Place the `{}` on the next line, together with the method name.",
                            dot_str
                        ),
                    ));
                }
            }
        }
    }
}

/// Returns true if this node is a heredoc (string/interpolated string starting with <<).
fn is_heredoc_node(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(str_node) = node.as_interpolated_string_node() {
        if let Some(open) = str_node.opening_loc() {
            return open.as_slice().starts_with(b"<<");
        }
    }
    if let Some(str_node) = node.as_string_node() {
        if let Some(open) = str_node.opening_loc() {
            return open.as_slice().starts_with(b"<<");
        }
    }
    false
}

/// Returns the line number of the heredoc's closing delimiter, or None if not a heredoc.
fn receiver_heredoc_end_line(source: &SourceFile, node: &ruby_prism::Node<'_>) -> Option<usize> {
    // Use closing_loc() to find the heredoc's closing delimiter line.
    if let Some(s) = node.as_string_node() {
        let closing = s.closing_loc()?;
        Some(source.offset_to_line_col(closing.start_offset()).0)
    } else if let Some(s) = node.as_interpolated_string_node() {
        let closing = s.closing_loc()?;
        Some(source.offset_to_line_col(closing.start_offset()).0)
    } else {
        None
    }
}

/// Recursively finds the effective receiver end line for a call chain.
/// When a call's receiver is itself a call whose receiver is a heredoc,
/// we need to trace through the chain to find the heredoc end.
fn call_chain_heredoc_end_line(source: &SourceFile, node: &ruby_prism::Node<'_>) -> Option<usize> {
    // If this node is a heredoc, return its end line
    if is_heredoc_node(node) {
        return receiver_heredoc_end_line(source, node);
    }

    // Otherwise if this is a call node, recurse into its receiver
    if let Some(call) = node.as_call_node() {
        if let Some(receiver) = call.receiver() {
            return call_chain_heredoc_end_line(source, &receiver);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(DotPosition, "cops/layout/dot_position");

    fn trailing_config() -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("trailing".to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn offense_trailing() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &DotPosition,
            include_bytes!("../../../tests/fixtures/cops/layout/dot_position/offense.trailing.rb"),
            trailing_config(),
        );
    }

    #[test]
    fn no_offense_trailing() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &DotPosition,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/dot_position/no_offense.trailing.rb"
            ),
            trailing_config(),
        );
    }
}
