use crate::cop::shared::node_type::{BLOCK_NODE, FORWARDING_SUPER_NODE, LAMBDA_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Layout/SpaceBeforeBlockBraces
///
/// ## Investigation findings
/// - **Tab whitespace FPs (17 FPs):** The "space" style check only looked for `b' '`
///   before `{`, causing false positives when tab characters were used for visual
///   alignment (e.g., `method_call\t\t\t{ block }`). RuboCop treats any whitespace
///   as satisfying the "space" requirement. Fixed by also accepting `b'\t'`.
/// - **Lambda literal FNs (647 FNs):** The cop only handled `BlockNode` but not
///   `LambdaNode`. In Prism, `-> { }` parses as a `LambdaNode`, not a `BlockNode`.
///   RuboCop's `on_block` also handles lambdas via `on_numblock`/`on_itblock` aliases,
///   since in Parser AST lambdas are block nodes. Fixed by also handling `LambdaNode`.
/// - **ForwardingSuperNode FNs (51 FNs, no_space variant):** `super { ... }` (without
///   explicit arguments) parses as `ForwardingSuperNode` in Prism, whose block child
///   is visited via `visit_block_node()` (named method) rather than `visit()` (generic
///   dispatch), so `visit_branch_node_enter` is never called for the inner BlockNode.
///   Fixed by registering for `FORWARDING_SUPER_NODE` and extracting the block.
/// - **No-space variant whitespace-range mismatches (7 FP, 24 FN):** RuboCop's
///   `range_with_surrounding_space(left_brace)` reports the start of the entire
///   left whitespace span, not just the byte immediately before `{`. That matters
///   for tab-aligned blocks (`foo\t\t{ ... }`) and backslash continuations
///   where `call \` ends one line and `{ ... }` starts the next. RuboCop flags
///   the first tab or the end of the previous line. Fixed by scanning the same
///   left whitespace range for `no_space`.
pub struct SpaceBeforeBlockBraces;

impl Cop for SpaceBeforeBlockBraces {
    fn name(&self) -> &'static str {
        "Layout/SpaceBeforeBlockBraces"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        // ForwardingSuperNode is needed because Prism's generated visitor calls
        // visit_block_node() (named method) instead of visit() (generic dispatch)
        // for its block child, so BlockNode inside ForwardingSuperNode never triggers
        // visit_branch_node_enter. We register for it and extract the block manually.
        &[BLOCK_NODE, LAMBDA_NODE, FORWARDING_SUPER_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "space");
        let empty_style = config.get_str("EnforcedStyleForEmptyBraces", "space");

        // Extract opening/closing from BlockNode, LambdaNode, or ForwardingSuperNode's block
        let (opening, closing) = if let Some(block) = node.as_block_node() {
            (block.opening_loc(), block.closing_loc())
        } else if let Some(lambda) = node.as_lambda_node() {
            (lambda.opening_loc(), lambda.closing_loc())
        } else if let Some(fwd_super) = node.as_forwarding_super_node() {
            if let Some(block) = fwd_super.block() {
                (block.opening_loc(), block.closing_loc())
            } else {
                return;
            }
        } else {
            return;
        };

        // Only check { blocks, not do...end
        if opening.as_slice() != b"{" {
            return;
        }

        let bytes = source.as_bytes();
        let before = opening.start_offset();

        // Check if this is an empty block {}
        let is_empty = closing.start_offset() == opening.end_offset();

        // Use empty_style for empty braces, style for non-empty
        let effective_style = if is_empty { empty_style } else { style };

        match effective_style {
            "no_space" => {
                if let Some(space_start) = no_space_whitespace_range_start(bytes, before) {
                    let (line, column) = source.offset_to_line_col(space_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space detected to the left of {.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: space_start,
                            end: before,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
            _ => {
                // "space" (default)
                // Accept any whitespace (space or tab) before the brace.
                // Tab characters are used for visual alignment in some codebases.
                if before > 0 && bytes[before - 1] != b' ' && bytes[before - 1] != b'\t' {
                    let (line, column) = source.offset_to_line_col(before);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space missing to the left of {.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: before,
                            end: before,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
        }
    }
}

fn no_space_whitespace_range_start(bytes: &[u8], brace_start: usize) -> Option<usize> {
    if brace_start == 0 {
        return None;
    }

    let mut start = brace_start;

    while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
        start -= 1;
    }

    while start > 0 && bytes[start - 1] == b'\n' {
        start -= 1;
    }

    (start < brace_start).then_some(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        SpaceBeforeBlockBraces,
        "cops/layout/space_before_block_braces"
    );
    crate::cop_autocorrect_fixture_tests!(
        SpaceBeforeBlockBraces,
        "cops/layout/space_before_block_braces"
    );

    #[test]
    fn no_space_style_flags_space() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("no_space".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"items.each { |x| puts x }\n";
        let diags = run_cop_full_with_config(&SpaceBeforeBlockBraces, src, config);
        assert_eq!(
            diags.len(),
            1,
            "no_space style should flag space before brace"
        );
        assert!(diags[0].message.contains("detected"));
    }

    #[test]
    fn no_space_style_accepts_no_space() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("no_space".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"items.each{ |x| puts x }\n";
        assert_cop_no_offenses_full_with_config(&SpaceBeforeBlockBraces, src, config);
    }

    #[test]
    fn no_space_offense_fixture() {
        use crate::testutil::assert_cop_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("no_space".into()),
                ),
                (
                    "EnforcedStyleForEmptyBraces".into(),
                    serde_yml::Value::String("no_space".into()),
                ),
            ]),
            ..CopConfig::default()
        };
        assert_cop_offenses_full_with_config(
            &SpaceBeforeBlockBraces,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_before_block_braces/no_space_offense.rb"
            ),
            config,
        );
    }

    #[test]
    fn no_space_no_offense_fixture() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("no_space".into()),
                ),
                (
                    "EnforcedStyleForEmptyBraces".into(),
                    serde_yml::Value::String("no_space".into()),
                ),
            ]),
            ..CopConfig::default()
        };
        assert_cop_no_offenses_full_with_config(
            &SpaceBeforeBlockBraces,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_before_block_braces/no_space_no_offense.rb"
            ),
            config,
        );
    }

    #[test]
    fn empty_braces_no_space_style() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForEmptyBraces".into(),
                serde_yml::Value::String("no_space".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"items.each {}\n";
        let diags = run_cop_full_with_config(&SpaceBeforeBlockBraces, src, config);
        assert_eq!(
            diags.len(),
            1,
            "no_space for empty braces should flag space before brace"
        );
    }
}
