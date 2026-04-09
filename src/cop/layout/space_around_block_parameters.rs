use crate::cop::shared::node_type::{BLOCK_NODE, FORWARDING_SUPER_NODE, LAMBDA_NODE};
use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-10)
///
/// CI baseline reported FP=3, FN=126.
///
/// The sampled FP fell into two shapes:
/// - empty block parameters written as `| |`, which RuboCop ignores;
/// - multiline parameter pipes where the closing `|` is on its own line and
///   the indentation before that pipe was being mistaken for "space after last
///   block parameter".
///
/// The dominant FN family was the missing `space after closing |` check on
/// single-line blocks such as `proc {|s|cmd.call s}` and `map{|x|...}`.
///
/// This pass switches the pipe checks to span-based whitespace handling:
/// newline-containing gaps are left to `Layout/MultilineBlockLayout`, empty
/// `| |` is skipped, and same-line `|body` now reports the missing space after
/// the closing pipe.
///
/// ## Corpus investigation (2026-03-14)
///
/// Remaining FN=20, all "Space before first block parameter detected."
/// Root cause: the cop did not handle `LambdaNode` (stabby lambdas with
/// `()` delimiters). RuboCop's `on_block` handles both block and lambda
/// nodes and checks `()` delimiters for lambdas. Added `LAMBDA_NODE` to
/// interested node types and handle `(` `)` delimiters.
///
/// Also added "Extra space before block parameter detected." check for
/// individual arguments (RuboCop's `check_each_arg`), which was missing
/// entirely — this detects extra whitespace before non-first args like
/// `|x,   y|`.
///
/// ## Corpus investigation (2026-03-15)
///
/// Remaining FN=18 from missing recursive descent into destructured (mlhs)
/// parameter groups. RuboCop's `check_arg` recurses into `mlhs_type?` nodes
/// to check extra space inside patterns like `(x,  y)`. nitrocop's
/// `collect_param_locations` only collected top-level params, so inner params
/// of `MultiTargetNode` groups were never checked. Fix: recurse into
/// `MultiTargetNode` children via `collect_multi_target_locations`.
///
/// ## Block-local variable fix (2026-03-17)
///
/// Fixed 18 FNs from blocks with only block-local variables (|; foo|, |;a|).
/// Previous attempt (commit 19d87d7b, reverted ffa7be5a) replaced byte scanning
/// with AST positions globally, introducing 1,411 new FPs. The correct fix:
/// keep byte scanning for normal blocks, and only override first_non_ws/last_non_ws
/// with local variable positions when parameters() is None and locals() is non-empty.
/// This uses `locals_only_positions()` to populate `first_local_start`/`last_local_end`
/// in BlockInfo, which are applied as overrides in check_node before the style checks.
///
/// ## Variant style fix: EnforcedStyleInsidePipes=space (2026-04-09)
///
/// Fixed 4 FP and 105 FN in the `space` variant:
///
/// **FN (105)**: `ForwardingSuperNode` (bare `super` without explicit args, e.g.
/// `super do |x| ... end`) was invisible to the cop. Prism's Visit trait for
/// `ForwardingSuperNode` calls `visitor.visit_block_node(&block)` directly instead
/// of `visitor.visit(&block.as_node())`, which bypasses `visit_branch_node_enter`
/// and thus the cop dispatch table. Fix: register `FORWARDING_SUPER_NODE` in
/// `interested_node_types` and extract its block child manually in `check_node`.
///
/// **FP (4)**: In `space` mode, single-param blocks like `|double|` had both
/// "No space before first" and "No space after last" emitted. RuboCop deduplicates
/// these because both offenses target the same source range (`args.first == args.last`).
/// Fix: track `top_level_arg_count` in BlockInfo; when it's 1 and "before first" already
/// fired, suppress "after last" to match RuboCop's dedup behavior.
///
/// ## Multiline closing delimiter indentation fix (2026-04-09)
///
/// The remaining `space`-style FN came from multiline block parameters whose
/// closing delimiter sits on its own indented line:
///
/// ```ruby
/// shared_examples do |
///   a:,
///   b:
///   |
/// end
/// ```
///
/// RuboCop accepts a multiline closing delimiter only when it starts the line
/// immediately after the line break. If that delimiter line is indented,
/// RuboCop reports "Extra space after last block parameter detected." on the
/// indentation itself. nitrocop previously skipped every newline-containing
/// gap before the closing delimiter, which also skipped this indentation-only
/// offense. Fix: keep the multiline fast-path for aligned closing delimiters,
/// but in `space` style emit the extra-space offense when the delimiter line
/// contains only spaces or tabs before the closing `|`/`)`.
pub struct SpaceAroundBlockParameters;

/// Extracted info about a block or lambda's parameters and body.
struct BlockInfo {
    /// Byte offset right after the opening delimiter (| or ().
    inner_start: usize,
    /// Byte offset of the closing delimiter.
    inner_end: usize,
    /// The closing delimiter location (for "space after closing" check).
    closing_end_offset: usize,
    /// Start offset of the closing delimiter (for diagnostic location).
    closing_start_offset: usize,
    /// Body start offset (None if no body).
    body_start: Option<usize>,
    /// Whether the closing delimiter is `|` (blocks) vs `)` (lambdas).
    /// Only blocks get the "space after closing `|`" check.
    is_pipe_delimited: bool,
    /// Parameter nodes for per-arg extra-space checking.
    param_locations: Vec<(usize, usize)>,
    /// Start offset of the first block-local variable name, when the block
    /// has only locals and no regular parameters (e.g., `|; foo|`).
    /// Used to override `first_non_whitespace` so the "space before first"
    /// check sees the local var name (past the `;`), not the `;` itself.
    first_local_start: Option<usize>,
    /// End offset of the last block-local variable name (for "space after last" check).
    last_local_end: Option<usize>,
    /// Number of top-level arguments (params + locals). When 1, RuboCop deduplicates
    /// the "missing space before first" and "missing space after last" offenses
    /// because both target the same source range (args.first == args.last).
    top_level_arg_count: usize,
}

impl Cop for SpaceAroundBlockParameters {
    fn name(&self) -> &'static str {
        "Layout/SpaceAroundBlockParameters"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        // FORWARDING_SUPER_NODE is included because Prism's Visit trait
        // dispatches its block child via visit_block_node() rather than
        // visit(), which bypasses visit_branch_node_enter and thus the
        // cop dispatch table. We handle it here to extract the block manually.
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
        let style = config.get_str("EnforcedStyleInsidePipes", "no_space");

        let info = if let Some(block) = node.as_block_node() {
            extract_block_info(&block)
        } else if let Some(lambda) = node.as_lambda_node() {
            extract_lambda_info(&lambda)
        } else if let Some(fwd_super) = node.as_forwarding_super_node() {
            // ForwardingSuperNode's block child is not visited through the
            // normal dispatch path (Prism bug), so we extract it manually.
            fwd_super.block().and_then(|b| extract_block_info(&b))
        } else {
            return;
        };

        let Some(info) = info else {
            return;
        };

        let bytes = source.as_bytes();
        let inner_start = info.inner_start;
        let inner_end = info.inner_end;

        if inner_start > inner_end || inner_end > bytes.len() {
            return;
        }
        let Some(first_non_ws) =
            util::first_non_whitespace_offset(bytes, inner_start).filter(|&o| o < inner_end)
        else {
            return;
        };
        let Some(last_non_ws) = last_non_whitespace(bytes, inner_start, inner_end) else {
            return;
        };

        // For blocks with only block-local variables (|; foo|), override the
        // first/last positions to point at the local variable name rather than
        // the `;`. This way the space checks see "content between | and foo"
        // rather than "content between | and ;", matching RuboCop's behavior
        // where shadowarg children are the "first"/"last" arguments.
        let first_non_ws = info.first_local_start.unwrap_or(first_non_ws);
        let last_non_ws = info
            .last_local_end
            .map(|end| end.saturating_sub(1))
            .unwrap_or(last_non_ws);
        let trailing_start = last_non_ws + 1;

        match style {
            "no_space" => {
                if first_non_ws > inner_start
                    && !contains_line_break(bytes, inner_start, first_non_ws)
                {
                    let (line, col) = source.offset_to_line_col(inner_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Space before first block parameter detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: inner_start,
                            end: first_non_ws,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }

                if trailing_start < inner_end
                    && !contains_line_break(bytes, trailing_start, inner_end)
                {
                    let (line, col) = source.offset_to_line_col(trailing_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Space after last block parameter detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: trailing_start,
                            end: inner_end,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }

                // Check each individual arg for extra space before it
                // (RuboCop's check_each_arg / check_arg).
                self.check_each_arg_extra_space(
                    source,
                    bytes,
                    &info.param_locations,
                    diagnostics,
                    &mut corrections,
                );
            }
            "space" => {
                let opening_has_newline = contains_line_break(bytes, inner_start, first_non_ws);
                let mut emitted_before_first = false;
                if !opening_has_newline && first_non_ws == inner_start {
                    let (line, col) = source.offset_to_line_col(inner_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "No space before first block parameter detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: inner_start,
                            end: inner_start,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                    emitted_before_first = true;
                }

                if !opening_has_newline && first_non_ws > inner_start + 1 {
                    let extra_start = inner_start + 1;
                    let (line, col) = source.offset_to_line_col(extra_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Extra space before first block parameter detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: extra_start,
                            end: first_non_ws,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }

                let closing_has_newline = contains_line_break(bytes, trailing_start, inner_end);
                // RuboCop deduplicates offenses at the same source range.
                // When there's a single arg, "before first" and "after last"
                // both target args.first/args.last (the same node), so the
                // second offense is suppressed. Skip "after last" when we
                // already emitted "before first" for a single-arg block.
                let dedup_after_last = emitted_before_first && info.top_level_arg_count == 1;
                if !dedup_after_last && !closing_has_newline && trailing_start == inner_end {
                    let (line, col) = source.offset_to_line_col(inner_end);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "No space after last block parameter detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: inner_end,
                            end: inner_end,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }

                let extra_after_last = if !closing_has_newline && inner_end > trailing_start + 1 {
                    Some((trailing_start + 1, inner_end))
                } else {
                    multiline_closing_indent_range(bytes, trailing_start, inner_end)
                };
                if let Some((extra_start, extra_end)) = extra_after_last {
                    let (line, col) = source.offset_to_line_col(extra_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Extra space after last block parameter detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: extra_start,
                            end: extra_end,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }

                // Check each individual arg for extra space before it
                self.check_each_arg_extra_space(
                    source,
                    bytes,
                    &info.param_locations,
                    diagnostics,
                    &mut corrections,
                );
            }
            _ => {}
        }

        // "Space after closing `|` missing." — only for pipe-delimited blocks
        if info.is_pipe_delimited {
            let Some(body_start) = info.body_start else {
                return;
            };
            let after_closing_start = info.closing_end_offset;
            if after_closing_start > body_start
                || contains_line_break(bytes, after_closing_start, body_start)
            {
                return;
            }
            if after_closing_start == body_start {
                let (line, col) = source.offset_to_line_col(info.closing_start_offset);
                let mut diag = self.diagnostic(
                    source,
                    line,
                    col,
                    "Space after closing `|` missing.".to_string(),
                );
                if let Some(ref mut corr) = corrections {
                    corr.push(crate::correction::Correction {
                        start: body_start,
                        end: body_start,
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

impl SpaceAroundBlockParameters {
    /// Check each argument for extra whitespace before it (more than one space
    /// after a comma). This corresponds to RuboCop's `check_each_arg` which
    /// reports "Extra space before block parameter detected."
    fn check_each_arg_extra_space(
        &self,
        source: &SourceFile,
        bytes: &[u8],
        param_locations: &[(usize, usize)],
        diagnostics: &mut Vec<Diagnostic>,
        corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
    ) {
        for &(param_start, _param_end) in param_locations {
            // Look backwards from param_start for whitespace.
            // RuboCop checks `range_with_surrounding_space(side: :left)` and
            // reports if there's more than one space before the arg's start.
            // We scan backwards from param_start to find the extent of
            // whitespace, then check if the character before the whitespace
            // is a comma (or opening delimiter). Extra space = >1 space after comma.
            if param_start == 0 {
                continue;
            }
            let mut ws_start = param_start;
            while ws_start > 0 && matches!(bytes[ws_start - 1], b' ' | b'\t') {
                ws_start -= 1;
            }
            // The char before the whitespace should be a comma for this check
            if ws_start == 0 || bytes[ws_start - 1] != b',' {
                continue;
            }
            let space_len = param_start - ws_start;
            if space_len > 1 {
                // Extra space: report the range from (ws_start + 1) to param_start
                // (keeping one space, removing the rest)
                let extra_start = ws_start + 1;
                let (line, col) = source.offset_to_line_col(extra_start);
                let mut diag = self.diagnostic(
                    source,
                    line,
                    col,
                    "Extra space before block parameter detected.".to_string(),
                );
                if let Some(corr) = corrections {
                    corr.push(crate::correction::Correction {
                        start: extra_start,
                        end: param_start,
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
}

/// Extract block info from a BlockNode.
fn extract_block_info(block: &ruby_prism::BlockNode<'_>) -> Option<BlockInfo> {
    let params = block.parameters()?;
    let block_params = params.as_block_parameters_node()?;
    let opening_loc = block_params.opening_loc()?;
    if opening_loc.as_slice() != b"|" {
        return None;
    }
    let closing_loc = block_params.closing_loc()?;
    if closing_loc.as_slice() != b"|" {
        return None;
    }

    let param_locations = collect_param_locations(&block_params);
    let top_level_arg_count = count_top_level_args(&block_params);

    // When there are no regular parameters but block-local variables exist
    // (e.g., |; foo| or |;glark|), record the first/last local variable
    // positions so the space checks can use them instead of byte scanning
    // (which would find the `;` rather than the variable name).
    let (first_local_start, last_local_end) =
        locals_only_positions(&block_params, &param_locations);

    Some(BlockInfo {
        inner_start: opening_loc.end_offset(),
        inner_end: closing_loc.start_offset(),
        closing_end_offset: closing_loc.end_offset(),
        closing_start_offset: closing_loc.start_offset(),
        body_start: block.body().map(|b| b.location().start_offset()),
        is_pipe_delimited: true,
        param_locations,
        first_local_start,
        last_local_end,
        top_level_arg_count,
    })
}

/// Extract block info from a LambdaNode.
fn extract_lambda_info(lambda: &ruby_prism::LambdaNode<'_>) -> Option<BlockInfo> {
    let params = lambda.parameters()?;
    let block_params = params.as_block_parameters_node()?;
    let opening_loc = block_params.opening_loc()?;
    if opening_loc.as_slice() != b"(" {
        return None;
    }
    let closing_loc = block_params.closing_loc()?;
    if closing_loc.as_slice() != b")" {
        return None;
    }

    let param_locations = collect_param_locations(&block_params);
    let top_level_arg_count = count_top_level_args(&block_params);
    let (first_local_start, last_local_end) =
        locals_only_positions(&block_params, &param_locations);

    Some(BlockInfo {
        inner_start: opening_loc.end_offset(),
        inner_end: closing_loc.start_offset(),
        closing_end_offset: closing_loc.end_offset(),
        closing_start_offset: closing_loc.start_offset(),
        body_start: lambda.body().map(|b| b.location().start_offset()),
        is_pipe_delimited: false,
        param_locations,
        first_local_start,
        last_local_end,
        top_level_arg_count,
    })
}

/// Collect (start_offset, end_offset) for each parameter in the block_params.
/// Recursively descends into destructured (MultiTargetNode) parameters to check
/// inner args too, matching RuboCop's `check_arg` which recurses into `mlhs_type?`.
fn collect_param_locations(
    block_params: &ruby_prism::BlockParametersNode<'_>,
) -> Vec<(usize, usize)> {
    let Some(params_node) = block_params.parameters() else {
        return Vec::new();
    };

    let mut locations = Vec::new();

    // Collect all required, optional, rest, keyword, etc. parameters
    for p in params_node.requireds().iter() {
        locations.push((p.location().start_offset(), p.location().end_offset()));
        // Recurse into destructured params like (x, y)
        if let Some(mt) = p.as_multi_target_node() {
            collect_multi_target_locations(&mt, &mut locations);
        }
    }
    for p in params_node.optionals().iter() {
        locations.push((p.location().start_offset(), p.location().end_offset()));
    }
    if let Some(rest) = params_node.rest() {
        locations.push((rest.location().start_offset(), rest.location().end_offset()));
    }
    for p in params_node.posts().iter() {
        locations.push((p.location().start_offset(), p.location().end_offset()));
        if let Some(mt) = p.as_multi_target_node() {
            collect_multi_target_locations(&mt, &mut locations);
        }
    }
    for p in params_node.keywords().iter() {
        locations.push((p.location().start_offset(), p.location().end_offset()));
    }
    if let Some(kw_rest) = params_node.keyword_rest() {
        locations.push((
            kw_rest.location().start_offset(),
            kw_rest.location().end_offset(),
        ));
    }
    if let Some(block) = params_node.block() {
        locations.push((
            block.location().start_offset(),
            block.location().end_offset(),
        ));
    }

    // Sort by start offset so we process them in order
    locations.sort_by_key(|&(start, _)| start);
    locations
}

/// Recursively collect inner param locations from a destructured (MultiTargetNode) group.
/// E.g., for `(x, y)` this adds locations of `x` and `y` so extra-space checks apply.
fn collect_multi_target_locations(
    mt: &ruby_prism::MultiTargetNode<'_>,
    locations: &mut Vec<(usize, usize)>,
) {
    for target in mt.lefts().iter() {
        locations.push((
            target.location().start_offset(),
            target.location().end_offset(),
        ));
        if let Some(inner_mt) = target.as_multi_target_node() {
            collect_multi_target_locations(&inner_mt, locations);
        }
    }
    if let Some(rest) = mt.rest() {
        locations.push((rest.location().start_offset(), rest.location().end_offset()));
    }
    for target in mt.rights().iter() {
        locations.push((
            target.location().start_offset(),
            target.location().end_offset(),
        ));
        if let Some(inner_mt) = target.as_multi_target_node() {
            collect_multi_target_locations(&inner_mt, locations);
        }
    }
}

/// Count the number of top-level arguments (regular params + locals) in the block.
/// This matches RuboCop's `arguments.children.length` for dedup purposes:
/// when there's exactly one top-level arg, "before first missing" and "after last
/// missing" offenses target the same source range and RuboCop deduplicates them.
fn count_top_level_args(block_params: &ruby_prism::BlockParametersNode<'_>) -> usize {
    let mut count = 0;
    if let Some(params_node) = block_params.parameters() {
        count += params_node.requireds().iter().count();
        count += params_node.optionals().iter().count();
        if params_node.rest().is_some() {
            count += 1;
        }
        count += params_node.posts().iter().count();
        count += params_node.keywords().iter().count();
        if params_node.keyword_rest().is_some() {
            count += 1;
        }
        if params_node.block().is_some() {
            count += 1;
        }
    }
    count += block_params.locals().iter().count();
    count
}

/// When block_params has no regular parameters but has locals (e.g., `|; foo|`),
/// return the (first_local_start, last_local_end) from the locals list.
/// Returns (None, None) when there ARE regular parameters or no locals.
fn locals_only_positions(
    block_params: &ruby_prism::BlockParametersNode<'_>,
    param_locations: &[(usize, usize)],
) -> (Option<usize>, Option<usize>) {
    // Only activate when there are no regular params
    if !param_locations.is_empty() {
        return (None, None);
    }
    let locals = block_params.locals();
    if locals.is_empty() {
        return (None, None);
    }
    let first = locals.iter().next().unwrap();
    let last = locals.iter().last().unwrap();
    (
        Some(first.location().start_offset()),
        Some(last.location().end_offset()),
    )
}

fn last_non_whitespace(bytes: &[u8], start: usize, end: usize) -> Option<usize> {
    (start..end)
        .rev()
        .find(|&idx| !matches!(bytes[idx], b' ' | b'\t' | b'\n' | b'\r'))
}

fn contains_line_break(bytes: &[u8], start: usize, end: usize) -> bool {
    bytes[start..end]
        .iter()
        .any(|&b| matches!(b, b'\n' | b'\r'))
}

fn multiline_closing_indent_range(
    bytes: &[u8],
    start: usize,
    end: usize,
) -> Option<(usize, usize)> {
    if start >= end {
        return None;
    }

    let last_line_break = bytes[start..end]
        .iter()
        .rposition(|&b| matches!(b, b'\n' | b'\r'))?;
    let indent_start = start + last_line_break + 1;
    if indent_start >= end {
        return None;
    }

    bytes[indent_start..end]
        .iter()
        .all(|&b| matches!(b, b' ' | b'\t'))
        .then_some((indent_start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        SpaceAroundBlockParameters,
        "cops/layout/space_around_block_parameters"
    );
    crate::cop_autocorrect_fixture_tests!(
        SpaceAroundBlockParameters,
        "cops/layout/space_around_block_parameters"
    );

    fn space_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyleInsidePipes".into(),
                serde_yml::Value::String("space".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn space_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &SpaceAroundBlockParameters,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_around_block_parameters/space_offense.rb"
            ),
            space_config(),
        );
    }

    #[test]
    fn space_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &SpaceAroundBlockParameters,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_around_block_parameters/space_no_offense.rb"
            ),
            space_config(),
        );
    }
}
