use crate::cop::shared::node_type::{BLOCK_NODE, FORWARDING_SUPER_NODE, LAMBDA_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Layout/SpaceInsideBlockBraces checks that block braces have or don't have
/// surrounding space inside them based on configuration.
///
/// ## Investigation notes
///
/// FN root causes identified:
/// 1. Contentful multiline `{ ... }` blocks without block parameters were not
///    checking the opening brace. RuboCop still flags missing left-brace space
///    in cases like `{<<-CODE`, `{{`, `{[`, `{FlavourSaver...`, and `{"`.
/// 2. Empty braces with multiple spaces (`{   }`) were not detected — only exactly
///    one space was checked. RuboCop flags any whitespace-only content inside braces.
/// 3. When SpaceBeforeBlockParameters=true (default) and `{|x|` is used, the cop
///    should flag "Space between { and | missing." but was instead falling through
///    to the generic "Space missing inside {." check, which has a different message
///    and location.
/// 4. Lambda literals (`-> { }`) parse as `LambdaNode` in Prism, not `BlockNode`.
///    The cop was not handling `LambdaNode` at all, causing 743 FNs across the
///    corpus. Repos like graphql-ruby (255 FNs), natalie (82), vagrant (43), and
///    danbooru (34) use lambda blocks heavily. Fixed by also handling `LAMBDA_NODE`.
/// 5. Bare `super { ... }` parses as `ForwardingSuperNode` with an attached
///    `BlockNode`. Prism's generated visitor walks that child via
///    `visit_block_node(&node)` instead of generic `visit(&node.as_node())`, so
///    AST cops listening only for `BLOCK_NODE` never receive the forwarded block.
///    That caused missed brace-spacing offenses like `super {|x| ... }` and
///    `super {['a', 'b']}`. Fixed by also listening on `FORWARDING_SUPER_NODE`
///    and reusing the attached block's brace and parameter locations.
/// 6. Single-line blocks ending with a tab before `}` (for example
///    `it { expect(foo).to eq(bar)\t}`) were falsely flagged because the
///    right-brace check only treated a literal space byte as valid inner
///    whitespace. RuboCop accepts tabs here, so the trailing-whitespace scan
///    now recognizes any ASCII whitespace before `}` and removes the full run
///    when `EnforcedStyle: no_space`.
/// 7. Multiline blocks with `EnforcedStyle: no_space` were not checking the right
///    brace for trailing space. The cop only checked single-line blocks for right
///    brace spacing. RuboCop checks multiline blocks but has an "alignment
///    exception" for properly aligned closing braces. Fixed by implementing the
///    alignment check: opening brace column must match either closing brace column
///    OR trailing space count on the last line of inner content.
pub struct SpaceInsideBlockBraces;

impl Cop for SpaceInsideBlockBraces {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideBlockBraces"
    }

    fn interested_node_types(&self) -> &'static [u8] {
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
        // Extract opening/closing/body/parameters from a directly visited BlockNode,
        // LambdaNode, or a ForwardingSuperNode's attached block.
        let (opening, closing, block_body_empty, has_params, params_location) =
            if let Some(block) = node.as_block_node() {
                (
                    block.opening_loc(),
                    block.closing_loc(),
                    block.body().is_none(),
                    block.parameters().is_some(),
                    block.parameters().map(|p| p.location()),
                )
            } else if let Some(lambda) = node.as_lambda_node() {
                (
                    lambda.opening_loc(),
                    lambda.closing_loc(),
                    lambda.body().is_none(),
                    lambda.parameters().is_some(),
                    lambda.parameters().map(|p| p.location()),
                )
            } else if let Some(forwarding_super) = node.as_forwarding_super_node() {
                let Some(block) = forwarding_super.block() else {
                    return;
                };
                (
                    block.opening_loc(),
                    block.closing_loc(),
                    block.body().is_none(),
                    block.parameters().is_some(),
                    block.parameters().map(|p| p.location()),
                )
            } else {
                return;
            };

        // Only check { } blocks, not do...end
        if opening.as_slice() != b"{" {
            return;
        }

        let bytes = source.as_bytes();
        let open_end = opening.end_offset();
        let close_start = closing.start_offset();

        let empty_style = config.get_str("EnforcedStyleForEmptyBraces", "no_space");
        let space_before_params = config.get_bool("SpaceBeforeBlockParameters", true);

        let (open_line, _) = source.offset_to_line_col(opening.start_offset());
        let (close_line, _) = source.offset_to_line_col(closing.start_offset());
        let is_multiline = open_line != close_line;

        let enforced = config.get_str("EnforcedStyle", "space");

        let params_inside_braces = has_params
            && params_location
                .as_ref()
                .is_some_and(|loc| loc.start_offset() >= open_end);

        // Handle empty blocks: {} or { } or {   }
        if block_body_empty {
            // Skip multiline empty blocks entirely (matches RuboCop behavior),
            // even when the block declares parameters like `proc {|x| ... }`.
            if is_multiline {
                return;
            }

            // Empty lambdas with parameters before the brace (`->(x) {}`) follow
            // the empty-brace style, while `proc {|x|}` still uses the block-param
            // spacing rules below.
            if !params_inside_braces {
                if close_start == open_end {
                    // Adjacent braces: {}
                    if empty_style == "space" {
                        let (line, column) = source.offset_to_line_col(opening.start_offset());
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space missing inside empty braces.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: open_end,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                    return;
                }

                // Check if the content between braces is whitespace-only
                let inner = &bytes[open_end..close_start];
                let is_whitespace_only = inner.iter().all(|&b| b == b' ' || b == b'\t');
                if is_whitespace_only {
                    if empty_style == "no_space" {
                        let (line, column) = source.offset_to_line_col(open_end);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside empty braces detected.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: close_start,
                                replacement: String::new(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                    return;
                }
            }
        }

        // For blocks with content: check left brace and parameters
        // Check left brace / space before block parameters
        // Only enter the pipe-checking branch if params are inside the braces
        // (e.g., `{ |x| ... }`). Lambda params with `()` come before the brace
        // (e.g., `->(x) { ... }`) and should use the no-params branch.
        if params_inside_braces {
            let pipe_start = params_location.as_ref().unwrap().start_offset();
            let space_after_open = open_end < pipe_start && bytes.get(open_end) == Some(&b' ');

            if space_after_open {
                // There IS a space between { and |
                if !space_before_params {
                    // SpaceBeforeBlockParameters: false — flag the space
                    let (line, column) = source.offset_to_line_col(open_end);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space between { and | detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: pipe_start,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
                // If space_before_params is true, space between { and | is correct — no offense
            } else if pipe_start == open_end {
                // No space: {|x| — directly adjacent
                if space_before_params {
                    // SpaceBeforeBlockParameters: true — flag missing space
                    let (line, column) = source.offset_to_line_col(opening.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space between { and | missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: open_end,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
                // If space_before_params is false, {| is correct — no offense on left brace
            }
        } else {
            // No params — RuboCop checks the left brace for both single-line and
            // multiline blocks with content. Newlines count as whitespace.
            let leading_whitespace_len = bytes[open_end..close_start]
                .iter()
                .take_while(|b| b.is_ascii_whitespace())
                .count();
            let has_space_after_open = leading_whitespace_len > 0;

            match enforced {
                "space" => {
                    if !has_space_after_open {
                        let (line, column) = source.offset_to_line_col(opening.start_offset());
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space missing inside {.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: open_end,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
                "no_space" => {
                    if has_space_after_open {
                        let (line, column) = source.offset_to_line_col(open_end);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside { detected.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: open_end + leading_whitespace_len,
                                replacement: String::new(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
                _ => {}
            }
        }

        // Check right brace spacing
        let trailing_whitespace_len = bytes[..close_start]
            .iter()
            .rev()
            .take_while(|b| b.is_ascii_whitespace())
            .count();

        let enforced = config.get_str("EnforcedStyle", "space");

        // For multiline blocks, check alignment before applying style rules
        let is_aligned = if is_multiline {
            let (_open_line, open_col) = source.offset_to_line_col(opening.start_offset());
            let (_, close_col) = source.offset_to_line_col(closing.start_offset());
            let inner = &bytes[open_end..close_start];

            // Count trailing spaces on the last line of inner content
            let inner_last_space_count =
                if let Some(last_newline_pos) = inner.iter().rposition(|&b| b == b'\n') {
                    let last_line = &inner[last_newline_pos + 1..];
                    last_line.iter().rev().take_while(|&&b| b == b' ').count()
                } else {
                    // Single-line inner content
                    inner.iter().rev().take_while(|&&b| b == b' ').count()
                };

            open_col == close_col || open_col == inner_last_space_count
        } else {
            false
        };

        match enforced {
            "space" => {
                // Only check single-line blocks for missing space
                if !is_multiline && trailing_whitespace_len == 0 {
                    let (line, column) = source.offset_to_line_col(closing.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space missing inside }.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: close_start,
                            end: close_start,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
            "no_space" => {
                if trailing_whitespace_len > 0 {
                    // For multiline blocks, only flag if NOT aligned
                    // (aligned multiline blocks are allowed to have trailing space)
                    if is_multiline && is_aligned {
                        // Aligned multiline - allow trailing space
                    } else {
                        let whitespace_start = close_start - trailing_whitespace_len;
                        let (line, column) = source.offset_to_line_col(whitespace_start);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside } detected.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: whitespace_start,
                                end: close_start,
                                replacement: String::new(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
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

    crate::cop_fixture_tests!(
        SpaceInsideBlockBraces,
        "cops/layout/space_inside_block_braces"
    );
    crate::cop_autocorrect_fixture_tests!(
        SpaceInsideBlockBraces,
        "cops/layout/space_inside_block_braces"
    );

    #[test]
    fn empty_braces_space_style_flags_no_space() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForEmptyBraces".into(),
                serde_yml::Value::String("space".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"items.each {}\n";
        let diags = run_cop_full_with_config(&SpaceInsideBlockBraces, src, config);
        assert_eq!(
            diags.len(),
            1,
            "space style for empty braces should flag braces"
        );
        assert!(diags[0].message.contains("missing"));
    }

    #[test]
    fn space_before_block_params_false_flags_space() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "SpaceBeforeBlockParameters".into(),
                serde_yml::Value::Bool(false),
            )]),
            ..CopConfig::default()
        };
        let src = b"items.each { |x| puts x }\n";
        let diags = run_cop_full_with_config(&SpaceInsideBlockBraces, src, config);
        assert!(
            diags.iter().any(|d| d.message.contains("{ and |")),
            "SpaceBeforeBlockParameters:false should flag space between {{ and |"
        );
    }

    #[test]
    fn multiline_block_with_params_missing_space() {
        use crate::testutil::run_cop_full;
        let src = b"items.each {|x|\n  puts x\n}\n";
        let diags = run_cop_full(&SpaceInsideBlockBraces, src);
        assert_eq!(diags.len(), 1, "should flag missing space between {{ and |");
        assert!(diags[0].message.contains("{ and |"));
    }

    #[test]
    fn multiline_block_with_params_has_space() {
        use crate::testutil::run_cop_full;
        let src = b"items.each { |x|\n  puts x\n}\n";
        let diags = run_cop_full(&SpaceInsideBlockBraces, src);
        assert!(
            diags.is_empty(),
            "should not flag multiline block with space before params"
        );
    }

    #[test]
    fn multiline_empty_block_no_offense() {
        use crate::testutil::run_cop_full;
        let src = b"items.each {\n}\n";
        let diags = run_cop_full(&SpaceInsideBlockBraces, src);
        assert!(diags.is_empty(), "should not flag multiline empty blocks");
    }

    #[test]
    fn multiline_empty_block_with_params_no_offense() {
        use crate::testutil::run_cop_full;
        let src = b"proc {|x|\n}\n";
        let diags = run_cop_full(&SpaceInsideBlockBraces, src);
        assert!(
            diags.is_empty(),
            "should not flag multiline empty blocks with parameters"
        );
    }

    #[test]
    fn empty_lambda_with_params_no_offense() {
        use crate::testutil::run_cop_full;
        let src = b"handler = ->(x) {}\n";
        let diags = run_cop_full(&SpaceInsideBlockBraces, src);
        assert!(
            diags.is_empty(),
            "should not flag empty lambdas with params before braces"
        );
    }

    #[test]
    fn empty_braces_multiple_spaces() {
        use crate::testutil::run_cop_full;
        let src = b"items.each {   }\n";
        let diags = run_cop_full(&SpaceInsideBlockBraces, src);
        assert_eq!(
            diags.len(),
            1,
            "should flag empty braces with multiple spaces"
        );
        assert!(diags[0].message.contains("empty braces detected"));
    }
}
