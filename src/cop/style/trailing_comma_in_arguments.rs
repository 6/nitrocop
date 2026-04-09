use crate::cop::shared::node_type::{BLOCK_ARGUMENT_NODE, CALL_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

use super::trailing_comma;

/// ## Investigation (2026-03-03)
///
/// Found 12 FPs with `EnforcedStyleForMultiline: comma`. Root cause: Prism
/// collapses keyword args into a single KeywordHashNode. The
/// `no_elements_on_same_line` check iterated over top-level args, so with
/// 1 KeywordHashNode the consecutive-pairs check vacuously passed. Fix: expand
/// KeywordHashNode into individual assoc elements for line comparisons (dc856393).
///
/// Investigation (2026-03-29)
///
/// Root cause of 480 FNs: when any call argument contained a heredoc, the cop
/// scanned from the last argument end offset all the way to `)`. In Prism that
/// range includes heredoc body text, so opener-line commas such as
/// `<<~GRAPHQL,` or `body: <<~BODY,` were rejected as if they were content.
/// Fix: mirror RuboCop's heredoc path and, when any argument contains a heredoc,
/// only search for a trailing comma on the same opener line.
///
/// Investigation (2026-03-30)
///
/// Root cause of 8 FNs: `is_heredoc_argument` did not recurse into explicit
/// `HashNode` values (e.g., `{ text: <<-END }`), only `KeywordHashNode`. When a
/// heredoc was nested inside a hash literal, `has_heredoc` was false, causing the
/// scanner to read through heredoc body content and miss the trailing comma.
/// Fix: add `HashNode` handling to `is_heredoc_argument`.
///
/// ## Investigation (2026-04-09)
///
/// Root cause of variant style divergence:
///
/// 1. `diff_comma` style was completely unimplemented — fell through to `no_comma`
///    behavior, causing 34,724 FP (flagging valid trailing commas as "Avoid") and
///    95,688 FN (missing "Put a comma" for multiline calls where last item
///    precedes newline). Fix: add `diff_comma` arm using
///    `last_item_precedes_newline` to determine `should_have_comma`.
///
/// 2. `comma` and `consistent_comma` styles only handled the "Put a comma" path
///    but never the "Avoid comma" path. When a trailing comma was present but
///    `should_have_comma` was false (e.g., single-line call, elements sharing a
///    line), nitrocop silently ignored it. Fix: add "Avoid comma... unless ..."
///    diagnostics for all non-default styles.
///
/// 3. `consistent_comma` was missing the braced hash exception from RuboCop's
///    `method_name_and_arguments_on_same_line?`. When the last argument is a
///    braced hash `{...}` and the closing paren is on the same line as `}`,
///    RuboCop considers the call not multiline for trailing comma purposes.
///    Fix: check if last raw arg is a `HashNode` (braced hash).
pub struct TrailingCommaInArguments;

impl Cop for TrailingCommaInArguments {
    fn name(&self) -> &'static str {
        "Style/TrailingCommaInArguments"
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
        let call_node = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let closing_loc = match call_node.closing_loc() {
            Some(loc) => loc,
            None => return,
        };

        let arguments = match call_node.arguments() {
            Some(args) => args,
            None => return,
        };

        let arg_list = arguments.arguments();
        let last_arg = match arg_list.last() {
            Some(a) => a,
            None => return,
        };

        let last_end = last_arg.location().end_offset();
        let closing_start = closing_loc.start_offset();
        let bytes = source.as_bytes();
        let has_heredoc = arg_list
            .iter()
            .any(|arg| trailing_comma::is_heredoc_node(&arg));

        // Skip if there's a block argument (&block) between last arg and closing paren.
        // The comma before &block is a separator, not a trailing comma.
        if let Some(block) = call_node.block() {
            if block.as_block_argument_node().is_some() {
                return;
            }
        }

        // Check for a trailing comma between the last argument and closing paren.
        if closing_start > bytes.len() {
            return;
        }

        // Arguments uses a stricter comma check that rejects any non-whitespace
        // content (unlike the simpler has_trailing_comma used by array/hash).
        // For heredocs, only check on the same line (no newline crossing).
        let has_comma = if last_end < closing_start {
            let search_range = &bytes[last_end..closing_start];
            if has_heredoc {
                is_only_horizontal_whitespace_and_comma(search_range)
            } else {
                trailing_comma::is_only_whitespace_and_comma(search_range)
            }
        } else {
            false
        };

        let style = config.get_str("EnforcedStyleForMultiline", "no_comma");

        // Determine if the call is multiline and whether a trailing comma should be present
        let close_line = source.offset_to_line_col(closing_start).0;
        let call_start_line = source
            .offset_to_line_col(call_node.location().start_offset())
            .0;
        let call_is_multiline = close_line > call_start_line;

        // Expand KeywordHashNode to count individual keyword args.
        let elem_locs = trailing_comma::effective_element_locations(arg_list.iter());
        let effective_args = elem_locs.len();

        // Helper: find trailing comma offset for diagnostics.
        let find_comma_offset = || {
            trailing_comma::find_trailing_comma_offset(bytes, last_end, closing_start, has_heredoc)
        };

        // Build the "Avoid comma" message with style-specific suffix.
        let avoid_msg = match style {
            "comma" => {
                "Avoid comma after the last parameter of a method call, unless each item is on its own line."
            }
            "consistent_comma" => {
                "Avoid comma after the last parameter of a method call, unless items are split onto multiple lines."
            }
            "diff_comma" => {
                "Avoid comma after the last parameter of a method call, unless that item immediately precedes a newline."
            }
            _ => "Avoid comma after the last parameter of a method call.",
        };

        // Single element with closing bracket on same line as element end:
        // RuboCop's allowed_multiline_argument? returns true, so multiline? is
        // false and should_have_comma is false for all styles. Only check for
        // unwanted commas.
        if effective_args == 1 {
            let last_arg_end_line = source.offset_to_line_col(last_end).0;
            if close_line == last_arg_end_line {
                if has_comma && last_end < closing_start {
                    if let Some(abs_offset) = find_comma_offset() {
                        let (line, column) = source.offset_to_line_col(abs_offset);
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            avoid_msg.to_string(),
                        ));
                    }
                }
                return;
            }
        }

        // Compute should_have_comma based on style, mirroring RuboCop's
        // should_have_comma?(style, node) method.
        let should_have = match style {
            "comma" => {
                // multiline?(node) && no_elements_on_same_line?(node)
                call_is_multiline
                    && trailing_comma::no_elements_on_same_line(source, &elem_locs, closing_start)
            }
            "consistent_comma" => {
                // multiline?(node) && !method_name_and_arguments_on_same_line?(node)
                if !call_is_multiline {
                    false
                } else {
                    let last_arg_end_line = source.offset_to_line_col(last_end).0;
                    // RuboCop: return false if node.last_line != node.last_argument.last_line
                    // (i.e., closing paren on different line from last arg end → NOT on same line)
                    if close_line != last_arg_end_line {
                        // Closing paren on different line from last arg end.
                        // method_name_and_arguments_on_same_line? returns false → should_have = true
                        true
                    } else {
                        // Closing paren on same line as last arg end.
                        // Check braced hash exception: if last raw arg is a HashNode (braced hash),
                        // RuboCop considers it "on same line" → should_have = false
                        if last_arg.as_hash_node().is_some() {
                            false
                        } else {
                            // Check method name line vs last arg end line
                            let method_line = call_node
                                .message_loc()
                                .map(|loc| source.offset_to_line_col(loc.start_offset()).0)
                                .unwrap_or(call_start_line);
                            method_line != last_arg_end_line
                        }
                    }
                }
            }
            "diff_comma" => {
                // multiline?(node) && last_item_precedes_newline?(node)
                call_is_multiline
                    && trailing_comma::last_item_precedes_newline(bytes, last_end, closing_start)
            }
            _ => false, // no_comma: never should have a trailing comma
        };

        if has_comma && !should_have {
            // Comma present but shouldn't be → "Avoid comma..."
            if let Some(abs_offset) = find_comma_offset() {
                let (line, column) = source.offset_to_line_col(abs_offset);
                diagnostics.push(self.diagnostic(source, line, column, avoid_msg.to_string()));
            }
        } else if !has_comma && should_have {
            // Comma missing but should be present → "Put a comma..."
            let (line, column) = source.offset_to_line_col(last_end);
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Put a comma after the last parameter of a multiline method call.".to_string(),
            ));
        }
    }
}

/// Like `is_only_whitespace_and_comma`, but stops at the first newline. This
/// matches RuboCop's heredoc-specific comma detection and avoids scanning into
/// heredoc bodies.
fn is_only_horizontal_whitespace_and_comma(bytes: &[u8]) -> bool {
    for &b in bytes {
        match b {
            b' ' | b'\t' => {}
            b',' => return true,
            b'\n' | b'\r' => return false,
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full_with_config;

    crate::cop_fixture_tests!(
        TrailingCommaInArguments,
        "cops/style/trailing_comma_in_arguments"
    );

    fn consistent_comma_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForMultiline".into(),
                serde_yml::Value::String("consistent_comma".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn consistent_comma_multiline_closing_on_same_line_as_last_arg() {
        // The closing paren is on the same line as the last arg, but the method name
        // is on a different line — this should require a trailing comma.
        let source = b"matching_token_for(\n  application, resource_owner, scopes, include_expired: false)\n";
        let diags =
            run_cop_full_with_config(&TrailingCommaInArguments, source, consistent_comma_config());
        assert_eq!(
            diags.len(),
            1,
            "consistent_comma should flag multiline call even when ) is on same line as last arg"
        );
    }

    #[test]
    fn consistent_comma_multiline_positional_args_closing_same_line() {
        // Same pattern but with only positional args (no keyword hash)
        let source = b"foo(\n  1, 2, 3)\n";
        let diags =
            run_cop_full_with_config(&TrailingCommaInArguments, source, consistent_comma_config());
        assert_eq!(
            diags.len(),
            1,
            "consistent_comma should flag multiline positional args"
        );
    }

    #[test]
    fn consistent_comma_single_line_no_offense() {
        let source = b"foo(1, 2, 3)\n";
        let diags =
            run_cop_full_with_config(&TrailingCommaInArguments, source, consistent_comma_config());
        assert!(
            diags.is_empty(),
            "Single line should not require trailing comma"
        );
    }

    #[test]
    fn consistent_comma_multiline_with_comma_no_offense() {
        let source = b"foo(\n  1,\n  2,\n)\n";
        let diags =
            run_cop_full_with_config(&TrailingCommaInArguments, source, consistent_comma_config());
        assert!(
            diags.is_empty(),
            "Multiline with trailing comma should be ok"
        );
    }

    fn comma_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForMultiline".into(),
                serde_yml::Value::String("comma".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn comma_style_args_on_same_line_no_offense() {
        // When multiple args share a line, comma style does NOT require trailing comma.
        // This matches RuboCop's no_elements_on_same_line? check.
        let source = b"not_change(\n  event.class, :count\n)\n";
        let diags = run_cop_full_with_config(&TrailingCommaInArguments, source, comma_config());
        assert!(
            diags.is_empty(),
            "comma style should not flag when args share a line"
        );
    }

    #[test]
    fn comma_style_each_arg_own_line_offense() {
        // When each arg is on its own line, comma style requires trailing comma.
        let source = b"not_change(\n  event.class,\n  :count\n)\n";
        let diags = run_cop_full_with_config(&TrailingCommaInArguments, source, comma_config());
        assert_eq!(
            diags.len(),
            1,
            "comma style should flag when each arg is on its own line"
        );
    }

    #[test]
    fn comma_style_keyword_args_sharing_line_no_offense() {
        // Keyword args form a single KeywordHashNode in Prism, but the
        // no_elements_on_same_line check must expand it to individual elements.
        let source =
            b"Retriable.retriable(\n  on: StandardError,\n  tries: 7, base_interval: 1.0\n)\n";
        let diags = run_cop_full_with_config(&TrailingCommaInArguments, source, comma_config());
        assert!(
            diags.is_empty(),
            "comma style should not flag when keyword args share a line"
        );
    }

    #[test]
    fn comma_style_keyword_args_each_own_line_offense() {
        // Each keyword arg on its own line — should require trailing comma.
        let source = b"foo(\n  on: StandardError,\n  tries: 7\n)\n";
        let diags = run_cop_full_with_config(&TrailingCommaInArguments, source, comma_config());
        assert_eq!(
            diags.len(),
            1,
            "comma style should flag when each keyword arg is on its own line"
        );
    }

    #[test]
    fn comma_style_mixed_args_keyword_sharing_line_no_offense() {
        // Positional arg + keyword args where keywords share a line
        let source = b"foo(\n  1,\n  a: 2, b: 3\n)\n";
        let diags = run_cop_full_with_config(&TrailingCommaInArguments, source, comma_config());
        assert!(
            diags.is_empty(),
            "comma style should not flag when keyword args share a line (mixed args)"
        );
    }

    #[test]
    fn offense_comma_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &TrailingCommaInArguments,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_arguments/offense.comma.rb"
            ),
            comma_config(),
        );
    }

    #[test]
    fn no_offense_comma_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArguments,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_arguments/no_offense.comma.rb"
            ),
            comma_config(),
        );
    }

    fn diff_comma_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForMultiline".into(),
                serde_yml::Value::String("diff_comma".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn offense_diff_comma_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &TrailingCommaInArguments,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_arguments/offense.diff_comma.rb"
            ),
            diff_comma_config(),
        );
    }

    #[test]
    fn no_offense_diff_comma_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArguments,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_arguments/no_offense.diff_comma.rb"
            ),
            diff_comma_config(),
        );
    }

    #[test]
    fn offense_consistent_comma_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &TrailingCommaInArguments,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_arguments/offense.consistent_comma.rb"
            ),
            consistent_comma_config(),
        );
    }

    #[test]
    fn no_offense_consistent_comma_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &TrailingCommaInArguments,
            include_bytes!(
                "../../../tests/fixtures/cops/style/trailing_comma_in_arguments/no_offense.consistent_comma.rb"
            ),
            consistent_comma_config(),
        );
    }

    #[test]
    fn consistent_comma_braced_hash_no_offense() {
        // Braced hash as last arg with closing paren on same line as } — RuboCop
        // considers this "method_name_and_arguments_on_same_line" and does NOT
        // require a trailing comma.
        let source = b"foo(arg, {\n  subject: \"hey\",\n  email: \"foo@bar.com\"\n})\n";
        let diags =
            run_cop_full_with_config(&TrailingCommaInArguments, source, consistent_comma_config());
        assert!(
            diags.is_empty(),
            "consistent_comma should not flag braced hash with ) on same line as }}"
        );
    }

    #[test]
    fn consistent_comma_single_line_trailing_comma_offense() {
        // Single-line call with trailing comma should flag "Avoid comma" even
        // with consistent_comma style.
        let source = b"foo(a, b, c,)\n";
        let diags =
            run_cop_full_with_config(&TrailingCommaInArguments, source, consistent_comma_config());
        assert_eq!(
            diags.len(),
            1,
            "consistent_comma should flag single-line trailing comma"
        );
        assert!(
            diags[0].message.contains("unless items are split"),
            "message should include consistent_comma suffix"
        );
    }

    #[test]
    fn diff_comma_multiline_last_precedes_newline_no_offense() {
        // Last arg precedes newline with comma — OK for diff_comma.
        let source = b"foo(\n  a,\n  b,\n)\n";
        let diags =
            run_cop_full_with_config(&TrailingCommaInArguments, source, diff_comma_config());
        assert!(
            diags.is_empty(),
            "diff_comma should accept comma when last item precedes newline"
        );
    }

    #[test]
    fn diff_comma_multiline_last_precedes_newline_missing_comma_offense() {
        // Last arg precedes newline without comma — should flag "Put a comma".
        let source = b"foo(\n  a,\n  b\n)\n";
        let diags =
            run_cop_full_with_config(&TrailingCommaInArguments, source, diff_comma_config());
        assert_eq!(
            diags.len(),
            1,
            "diff_comma should flag missing comma when last item precedes newline"
        );
    }

    #[test]
    fn diff_comma_single_line_trailing_comma_offense() {
        // Single-line with comma — should flag "Avoid comma".
        let source = b"foo(a, b, c,)\n";
        let diags =
            run_cop_full_with_config(&TrailingCommaInArguments, source, diff_comma_config());
        assert_eq!(
            diags.len(),
            1,
            "diff_comma should flag single-line trailing comma"
        );
        assert!(
            diags[0]
                .message
                .contains("unless that item immediately precedes a newline"),
            "message should include diff_comma suffix"
        );
    }
}
