use crate::cop::shared::node_type::{
    CALL_NODE, INDEX_AND_WRITE_NODE, INDEX_OPERATOR_WRITE_NODE, INDEX_OR_WRITE_NODE,
    INDEX_TARGET_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// ## Corpus investigation (2026-03-10)
///
/// Cached corpus oracle reported FP=12, FN=1.
///
/// Fixed FN=1: multiline empty brackets such as `items[\n ]` were treated as
/// non-empty because the empty-bracket check only accepted spaces/tabs and ran
/// after the multiline early return. Empty-bracket detection now treats CR/LF
/// as whitespace and runs before the multiline guard.
///
/// ## Corpus investigation (2026-03-13)
///
/// FP=9 across 3 repos: zammad (5), activemerchant (3), puppet (1). Two root
/// causes:
///
/// 1. **Multiline node skip (2 FPs):** RuboCop's `return if node.multiline?`
///    checks the entire send node span, not just the bracket span. For
///    `mail[ key ] = if ... end` and `memo[ key ] = { ... }`, the brackets are
///    on one line but the node spans multiple lines. Added a whole-node
///    multiline check.
///
/// 2. **Nested bracket selection (7 FPs):** RuboCop's token-based
///    `left_ref_bracket` method picks the first or last `tLBRACK2` token in
///    the node range. For `[]` (read) calls where arguments contain chained
///    brackets (e.g. `CONST[ resp[:x][:y] ]`) or the receiver has brackets
///    (e.g. `user['k'][ arg['id'] ]`), the outer brackets are never checked.
///    Matched that behavior by skipping outer-bracket checks in those cases.
///
/// ## Corpus investigation (2026-03-14)
///
/// FN=1: `v [0 ] += # comment\n  42` — multiline compound assignment where
/// brackets are single-line but the IndexOperatorWriteNode spans multiple
/// lines (the RHS value is on the next line). RuboCop's `on_send` receives
/// the inner `[]` send node (single-line), not the outer op_asgn. Fixed by
/// restricting the whole-node multiline skip to CallNode only; index write
/// nodes already have the bracket-span multiline check.
///
/// ## Corpus investigation (2026-03-31)
///
/// FP=2:
///
/// 1. `current_class_accessor[:table].header_description[ key[1..-1] ] = value`
/// 2. `app.extensions[:blog].find { ... }[ 1 ]`
///
/// Root cause: the previous implementation always inspected the current
/// call's own brackets for `[]`/`[]=`, but RuboCop first selects a
/// reference-bracket token anywhere in the call's token range. For `[]=`,
/// it chooses the first reference bracket in the node range. For `[]`, it
/// chooses the last one unless the token immediately before that `[` is not
/// `]`, in which case it falls back to the first. Matching that selection
/// logic fixes both false positives without suppressing the broader
/// offending patterns that RuboCop still reports.
///
/// ## Corpus investigation (2026-04-09)
///
/// Variant `EnforcedStyle: space` still had 165 FN on multi-write targets like
/// `data['stdout'], data['stderr'], status = ...` and `rt[col], message = ...`.
/// Prism parses those LHS references as `IndexTargetNode`, so the cop never
/// visited them. RuboCop handles them like `[]=`, selecting the first
/// reference bracket in the target subtree; this intentionally ignores outer
/// target brackets in cases like `user[ 'items' ][key], other = rhs`. Fixed by
/// adding `INDEX_TARGET_NODE` support and reusing first-bracket selection for
/// target nodes. Also report missing leading space for `EnforcedStyle: space`
/// at `[` to match RuboCop's offense location.
///
/// ## Corpus investigation (2026-04-09, follow-up)
///
/// Variant `EnforcedStyle: space` still diverged after the multi-write fix:
///
/// 1. `Try[StandardError] do ... end` was an FN because Prism's `CallNode`
///    location spans the trailing block, while RuboCop's `SendNode#multiline?`
///    does not. Block-attached `[]` reads must still be checked.
/// 2. Multiline receiver chains such as `sort { ... }.reverse[0..49]` were FPs
///    once the guard was removed entirely. RuboCop still skips those because
///    the `[]` send itself is multiline when the receiver spans multiple lines.
///
/// Match both behaviors by applying the whole-node multiline skip to `[]=`
/// calls and to `[]` reads without an attached block, while still checking
/// `[]` reads that own their trailing `do ... end`.
///
/// ## Corpus investigation (2026-04-09, heredoc interpolation)
///
/// Variant `EnforcedStyle: space` had 2 FN on
/// `mysql_query(<<-SQL).first["pagetext"]` where the heredoc body contains
/// interpolation with brackets like `#{mapping[post.id][0]}`. Prism includes
/// the heredoc content in the `CallNode` subtree, so `reference_bracket_pairs`
/// collected brackets from inside the heredoc interpolation. The bracket
/// selection logic then picked an inner bracket pair instead of the outer
/// `["pagetext"]` brackets. RuboCop's `tokens_within(node)` excludes heredoc
/// body tokens, so it always picks the correct brackets. Fixed by making
/// `ReferenceBracketCollector` skip `InterpolatedStringNode`s whose opening
/// delimiter starts with `<<` (i.e., heredocs).
pub struct SpaceInsideReferenceBrackets;

impl Cop for SpaceInsideReferenceBrackets {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideReferenceBrackets"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CALL_NODE,
            INDEX_TARGET_NODE,
            INDEX_AND_WRITE_NODE,
            INDEX_OPERATOR_WRITE_NODE,
            INDEX_OR_WRITE_NODE,
        ]
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
        let enforced_style = config.get_str("EnforcedStyle", "no_space");
        let empty_style = config.get_str("EnforcedStyleForEmptyBrackets", "no_space");

        let bytes = source.as_bytes();

        let (open_start, close_start) = match reference_bracket_offsets(node, bytes) {
            Some(offsets) => offsets,
            None => return,
        };
        let open_end = open_start + 1;

        // Check for empty brackets
        let is_empty = close_start == open_end
            || (close_start > open_end
                && bytes[open_end..close_start]
                    .iter()
                    .all(|&b| matches!(b, b' ' | b'\t' | b'\n' | b'\r')));

        if is_empty {
            match empty_style {
                "no_space" => {
                    if close_start > open_end {
                        let (line, col) = source.offset_to_line_col(open_end);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            col,
                            "Do not use space inside empty reference brackets.".to_string(),
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
                }
                "space" => {
                    if close_start == open_end || (close_start - open_end) != 1 {
                        let (line, col) = source.offset_to_line_col(open_start);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            col,
                            "Use one space inside empty reference brackets.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
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
                _ => {}
            }
            return;
        }

        // Skip multiline non-empty brackets (bracket span).
        let (open_line, _) = source.offset_to_line_col(open_start);
        let (close_line, _) = source.offset_to_line_col(close_start);
        if open_line != close_line {
            return;
        }

        // RuboCop skips multiline `[]=` sends such as `obj[key] = if\n...\nend`
        // and multiline `[]` reads whose receiver chain spans lines, but not
        // block-attached reads like `Try[StandardError] do ... end`. Prism's
        // CallNode location includes the trailing block, so distinguish whether
        // the current `[]` call owns an attached BlockNode.
        if let Some(call) = node.as_call_node() {
            let has_attached_block = call
                .block()
                .is_some_and(|block| block.as_block_node().is_some());
            let should_skip_multiline = call.name().as_slice() == b"[]="
                || (call.name().as_slice() == b"[]" && !has_attached_block);

            if should_skip_multiline {
                let node_start_line = source.offset_to_line_col(node.location().start_offset()).0;
                let node_end_line = source.offset_to_line_col(node.location().end_offset()).0;
                if node_start_line != node_end_line {
                    return;
                }
            }
        }

        let space_after_open = bytes.get(open_end) == Some(&b' ');
        let space_before_close = close_start > 0 && bytes.get(close_start - 1) == Some(&b' ');

        match enforced_style {
            "no_space" => {
                if space_after_open {
                    let (line, col) = source.offset_to_line_col(open_end);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Do not use space inside reference brackets.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: open_end + 1,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
                if space_before_close {
                    let (line, col) = source.offset_to_line_col(close_start - 1);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Do not use space inside reference brackets.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: close_start - 1,
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
            "space" => {
                if !space_after_open {
                    let (line, col) = source.offset_to_line_col(open_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Use space inside reference brackets.".to_string(),
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
                if !space_before_close {
                    let (line, col) = source.offset_to_line_col(close_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        col,
                        "Use space inside reference brackets.".to_string(),
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
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        SpaceInsideReferenceBrackets,
        "cops/layout/space_inside_reference_brackets"
    );
    crate::cop_autocorrect_fixture_tests!(
        SpaceInsideReferenceBrackets,
        "cops/layout/space_inside_reference_brackets"
    );

    fn space_config() -> CopConfig {
        use std::collections::HashMap;

        CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("space".into()),
                ),
                (
                    "EnforcedStyleForEmptyBrackets".into(),
                    serde_yml::Value::String("space".into()),
                ),
            ]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn space_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &SpaceInsideReferenceBrackets,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_inside_reference_brackets/space_offense.rb"
            ),
            space_config(),
        );
    }

    #[test]
    fn space_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &SpaceInsideReferenceBrackets,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_inside_reference_brackets/space_no_offense.rb"
            ),
            space_config(),
        );
    }
}

fn reference_bracket_offsets(node: &ruby_prism::Node<'_>, bytes: &[u8]) -> Option<(usize, usize)> {
    if let Some(call) = node.as_call_node() {
        return call_bracket_offsets(&call, bytes);
    }
    if let Some(index) = node.as_index_target_node() {
        return index_target_bracket_offsets(&index);
    }
    if let Some(index) = node.as_index_and_write_node() {
        return index_write_bracket_offsets(
            index.receiver(),
            index.opening_loc().start_offset(),
            index.closing_loc().start_offset(),
        );
    }
    if let Some(index) = node.as_index_operator_write_node() {
        return index_write_bracket_offsets(
            index.receiver(),
            index.opening_loc().start_offset(),
            index.closing_loc().start_offset(),
        );
    }
    if let Some(index) = node.as_index_or_write_node() {
        return index_write_bracket_offsets(
            index.receiver(),
            index.opening_loc().start_offset(),
            index.closing_loc().start_offset(),
        );
    }
    None
}

fn call_bracket_offsets(call: &ruby_prism::CallNode<'_>, bytes: &[u8]) -> Option<(usize, usize)> {
    let method_name = call.name().as_slice();
    if method_name != b"[]" && method_name != b"[]=" {
        return None;
    }
    call_reference_bracket_offsets(call)?;

    let pairs = reference_bracket_pairs(&call.as_node());
    let first = pairs.first().copied()?;
    if method_name == b"[]=" {
        return Some(first);
    }

    let last = pairs.last().copied()?;
    if previous_non_whitespace_byte(bytes, last.0) == Some(b']') {
        Some(last)
    } else {
        Some(first)
    }
}

fn index_target_bracket_offsets(index: &ruby_prism::IndexTargetNode<'_>) -> Option<(usize, usize)> {
    reference_bracket_pairs(&index.as_node()).first().copied()
}

fn index_write_bracket_offsets(
    receiver: Option<ruby_prism::Node<'_>>,
    open_start: usize,
    close_start: usize,
) -> Option<(usize, usize)> {
    receiver?;
    Some((open_start, close_start))
}

fn previous_non_whitespace_byte(bytes: &[u8], offset: usize) -> Option<u8> {
    bytes[..offset]
        .iter()
        .rev()
        .find(|&&b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        .copied()
}

fn call_reference_bracket_offsets(call: &ruby_prism::CallNode<'_>) -> Option<(usize, usize)> {
    let method_name = call.name().as_slice();
    if method_name != b"[]" && method_name != b"[]=" {
        return None;
    }

    let opening_loc = call.opening_loc()?;
    let closing_loc = call.closing_loc()?;
    if opening_loc.as_slice() != b"[" || closing_loc.as_slice() != b"]" {
        return None;
    }

    Some((opening_loc.start_offset(), closing_loc.start_offset()))
}

fn reference_bracket_pairs(node: &ruby_prism::Node<'_>) -> Vec<(usize, usize)> {
    let mut collector = ReferenceBracketCollector { pairs: Vec::new() };
    collector.visit(node);
    collector
        .pairs
        .sort_unstable_by_key(|(open_start, _)| *open_start);
    collector.pairs
}

struct ReferenceBracketCollector {
    pairs: Vec<(usize, usize)>,
}

impl<'pr> Visit<'pr> for ReferenceBracketCollector {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if let Some(offsets) = call_reference_bracket_offsets(node) {
            self.pairs.push(offsets);
        }

        ruby_prism::visit_call_node(self, node);
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        // Don't descend into heredoc content. RuboCop's `tokens_within(node)`
        // excludes heredoc body tokens, so bracket pairs inside heredoc
        // interpolation must not influence the outer bracket selection.
        if node
            .opening_loc()
            .is_some_and(|o| o.as_slice().starts_with(b"<<"))
        {
            return;
        }
        ruby_prism::visit_interpolated_string_node(self, node);
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        self.pairs.push((
            node.opening_loc().start_offset(),
            node.closing_loc().start_offset(),
        ));

        ruby_prism::visit_index_and_write_node(self, node);
    }

    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'pr>) {
        self.pairs.push((
            node.opening_loc().start_offset(),
            node.closing_loc().start_offset(),
        ));

        ruby_prism::visit_index_operator_write_node(self, node);
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        self.pairs.push((
            node.opening_loc().start_offset(),
            node.closing_loc().start_offset(),
        ));

        ruby_prism::visit_index_or_write_node(self, node);
    }

    fn visit_index_target_node(&mut self, node: &ruby_prism::IndexTargetNode<'pr>) {
        self.pairs.push((
            node.opening_loc().start_offset(),
            node.closing_loc().start_offset(),
        ));

        ruby_prism::visit_index_target_node(self, node);
    }
}
