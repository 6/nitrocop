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
/// - **ForwardingSuperNode FNs:** The cop's interested_node_types only included
///   `BlockNode` and `LambdaNode`, but `super { }` (without parentheses) parses as
///   `ForwardingSuperNode` in Prism. The AST walker was not visiting its child
///   `BlockNode`, so the cop was never invoked for these nodes. Fixed by adding
///   `FORWARDING_SUPER_NODE` to interested_node_types and handling it in check_node.
pub struct SpaceBeforeBlockBraces;

impl Cop for SpaceBeforeBlockBraces {
    fn name(&self) -> &'static str {
        "Layout/SpaceBeforeBlockBraces"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[BLOCK_NODE, FORWARDING_SUPER_NODE, LAMBDA_NODE]
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

        // Extract opening/closing from BlockNode, LambdaNode, or ForwardingSuperNode
        let (opening, closing) = if let Some(block) = node.as_block_node() {
            (block.opening_loc(), block.closing_loc())
        } else if let Some(lambda) = node.as_lambda_node() {
            (lambda.opening_loc(), lambda.closing_loc())
        } else if let Some(forwarding_super) = node.as_forwarding_super_node() {
            let block = match forwarding_super.block() {
                Some(block) => block,
                None => return,
            };
            (block.opening_loc(), block.closing_loc())
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
                if before > 0 && bytes[before - 1] == b' ' {
                    let (line, column) = source.offset_to_line_col(before - 1);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space detected to the left of {.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: before - 1,
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
    fn no_space_style_flags_super_block() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("no_space".into()),
            )]),
            ..CopConfig::default()
        };
        // super { } (without parentheses) parses as ForwardingSuperNode in Prism
        let src = b"def call; super { |args| puts args }; end\n";
        let diags = run_cop_full_with_config(&SpaceBeforeBlockBraces, src, config);
        assert_eq!(
            diags.len(),
            1,
            "no_space style should flag space before brace in super blocks"
        );
        assert!(diags[0].message.contains("detected"));
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
