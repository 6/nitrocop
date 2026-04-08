use crate::cop::shared::node_type::{BLOCK_NODE, FORWARDING_SUPER_NODE, LAMBDA_NODE, SUPER_NODE};
use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Location, Severity};
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-14)
///
/// FP=1: backslash line continuation before `do` (e.g. `method(arg) \\\n  do |x|`)
/// caused the blank line after `do` to be flagged. RuboCop uses
/// `send_node.last_line` as the reference, so the `do` line itself is the
/// "first body line" and the blank line is not adjacent to the opening.
/// Fix: walk backward through `\\`-continued lines to find the effective
/// first line of the block construct.
///
/// FN=6: lambda brace/do blocks (`-> (a) {`, `-> do`) were not checked
/// because the cop only visited `BLOCK_NODE`. Added `LAMBDA_NODE`.
/// Previous attempt (2026-03-10) regressed because it did not adjust
/// `keyword_offset` for backslash continuations simultaneously; this
/// combined fix resolves both.
///
/// FN=5 (2026-03-14): string concatenation with `\` spanning `it`/`describe`
/// blocks (e.g. `it 'str' \ 'str' do`). The previous `adjusted_keyword_offset`
/// always walked backward through `\` continuations, landing on the first
/// continuation line. For `it '...' \ '...' do`, this moved the reference to
/// the `it` line, making the check look at the continuation string line
/// (not blank) instead of the line after `do` (blank). Fix: only walk backward
/// when `do`/`{` is the first non-whitespace token on its line (i.e., `do` is
/// on a separate continuation line). When `do`/`{` has args before it on the
/// same line, use the `do` line directly — matching RuboCop's
/// `send_node.last_line` behavior.
///
/// ## Corpus investigation (2026-03-25)
///
/// FP=2: lambda blocks with multiline parameters (`-> (a:,\n b:) do\n\n body`)
/// were incorrectly flagging the blank line after `do` as "extra empty line at
/// block body beginning." Root cause: nitrocop used `opening_loc` (`do`/`{`) as
/// the reference line for all blocks, but RuboCop uses `send_node.last_line`
/// which for lambda blocks is the `->` operator line (not the `do` line). When
/// params span multiple lines, the `->` is on an earlier line, so the line after
/// `->` is a param continuation (not blank), and RuboCop does not flag it.
/// Fix: for `LambdaNode`, when `->` is on a different line than `do`/`{`, use
/// the `->` operator offset as the effective opening reference.
///
/// ## Corpus investigation (2026-04-08)
///
/// Variant-only divergence with `EnforcedStyle: empty_lines` came from two
/// narrow mismatches with RuboCop:
/// 1. Comment-only blocks (`do # comment end`) are treated as empty because
///    Prism reports `body: nil`, and RuboCop skips empty bodies for
///    `empty_lines`. nitrocop was still requiring blank lines at beginning/end.
/// 2. `super do ... end` blocks were missed because Prism exposes them through
///    `SuperNode` / `ForwardingSuperNode`, not plain `BlockNode`.
/// 3. Two-line blocks where exactly one delimiter shares a body line diverge
///    from the shared helper when the effective opening is the actual `do`/`{`
///    line: `it { foo.\n  bar }` and `items.each { |x|\n  puts x }` both need
///    the beginning offense only. Multiline `->` params are different because
///    RuboCop still uses the earlier `->` line as the opening reference, so
///    those cases continue through the normal missing-beginning/end checks.
///
/// ## Corpus investigation (2026-04-08, variant `empty_lines`)
///
/// FN=4, FP=4: backslash continuation before `do` (e.g.
/// `it "str" \ "str" \ do\n body\n end`) with `empty_lines` style.
/// `adjusted_keyword_offset` walked ALL the way back through `\` continuations
/// to the `it` line, making the check flag the continuation string line instead
/// of the `do` line. RuboCop uses `send_node.last_line` (the last argument
/// line, one line before `do`) as the reference. Fix: for `empty_lines` style,
/// only step back one line instead of walking all the way through `\`.
///
/// Remaining FP=18: `# rubocop:disable Layout:LineLength` (colon syntax)
/// is treated by RuboCop as a department-level disable (suppressing ALL
/// `Layout/*` cops including this one), but nitrocop's directive parser
/// in `src/parse/directives.rs` does not recognize single-colon department
/// syntax. This is a directive-parsing issue, not a cop detection bug.
/// Fix requires updating `normalize_directive_cop_name()` to treat
/// `Dept:CopName` → `Dept` (department disable), matching RuboCop's
/// `COP_NAME_PATTERN` which excludes `:` from `\w`.
pub struct EmptyLinesAroundBlockBody;

/// Compute the effective opening offset for empty-line checks.
///
/// RuboCop uses `send_node.last_line` as the reference — the last line of
/// the method call arguments. In Prism we don't have direct parent access,
/// so we approximate:
///
/// - If the `do`/`{` keyword has non-whitespace content before it on its
///   line (e.g. `'has not passed' do`), the arguments end on the same line
///   as `do`, so use the `do` line directly.
/// - If `do`/`{` is the first non-whitespace token on its line AND the
///   preceding line ends with `\`, then `do` was placed on a separate
///   continuation line (e.g. `run_command(arg) \ \n  do |x|`). Walk
///   backward through `\` continuations to find the method-call line and
///   use that as the reference.
/// Compute the effective opening offset for empty-line checks.
///
/// When `full_walkback` is true (used for `no_empty_lines` style), walk all
/// the way back through `\` continuations to the method-call line. This makes
/// the "line after effective opening" be the next continuation string, which is
/// never blank, so no false extra-blank-line offense.
///
/// When `full_walkback` is false (used for `empty_lines` style), only step
/// back one line. This matches RuboCop's `send_node.last_line`, which is the
/// last argument line — the line immediately before `do`/`{`. The helper then
/// checks the `do` line itself for emptiness, matching RuboCop exactly.
fn adjusted_keyword_offset(
    source: &SourceFile,
    opening_offset: usize,
    full_walkback: bool,
) -> usize {
    let (opening_line, opening_col) = source.offset_to_line_col(opening_offset);

    // Check if there is non-whitespace content before `do`/`{` on its line.
    let has_content_before = if let Some(line_bytes) = util::line_at(source, opening_line) {
        line_bytes[..opening_col]
            .iter()
            .any(|&b| b != b' ' && b != b'\t')
    } else {
        false
    };

    // If args are on the same line as `do`, use the `do` line — this is
    // `send_node.last_line` in RuboCop terms.
    if has_content_before {
        return opening_offset;
    }

    // Helper: check if a line (by 1-indexed number) ends with `\`.
    let line_ends_with_backslash = |line_num: usize| -> bool {
        if let Some(bytes) = util::line_at(source, line_num) {
            let mut end = bytes.len();
            while end > 0 && (bytes[end - 1] == b'\n' || bytes[end - 1] == b'\r') {
                end -= 1;
            }
            end > 0 && bytes[end - 1] == b'\\'
        } else {
            false
        }
    };

    if !full_walkback {
        // For `empty_lines` style: step back exactly one line when the
        // previous line ends with `\`. This matches `send_node.last_line`
        // in RuboCop (the last argument line, not the first continuation).
        if opening_line > 1 && line_ends_with_backslash(opening_line - 1) {
            if let Some(off) = source.line_col_to_offset(opening_line - 1, 0) {
                return off;
            }
        }
        return opening_offset;
    }

    // Full walkback for `no_empty_lines` style: walk backward through all
    // `\` continuations to find the method-call line.
    let mut line = opening_line;
    loop {
        if line <= 1 {
            break;
        }
        if line_ends_with_backslash(line - 1) {
            line -= 1;
            continue;
        }
        break;
    }
    if let Some(off) = source.line_col_to_offset(line, 0) {
        off
    } else {
        opening_offset
    }
}

fn missing_beginning_empty_line_diagnostic(
    cop_name: &'static str,
    source: &SourceFile,
    line: usize,
    mut corrections: Option<&mut Vec<crate::correction::Correction>>,
) -> Option<Diagnostic> {
    let line_bytes = util::line_at(source, line)?;
    if util::is_blank_line(line_bytes) {
        return None;
    }

    let mut diag = Diagnostic {
        path: source.path_str().to_string(),
        location: Location { line, column: 0 },
        severity: Severity::Convention,
        cop_name: cop_name.to_string(),
        message: "Empty line missing at block body beginning.".to_string(),
        corrected: false,
    };

    if let Some(ref mut corr) = corrections {
        if let Some(offset) = source.line_col_to_offset(line, 0) {
            corr.push(crate::correction::Correction {
                start: offset,
                end: offset,
                replacement: "\n".to_string(),
                cop_name,
                cop_index: 0,
            });
            diag.corrected = true;
        }
    }

    Some(diag)
}

fn check_empty_lines_style_with_rubocop_edge_cases(
    cop_name: &'static str,
    source: &SourceFile,
    effective_opening: usize,
    opening_offset: usize,
    closing_offset: usize,
    body_start_offset: Option<usize>,
    corrections: Option<&mut Vec<crate::correction::Correction>>,
) -> Vec<Diagnostic> {
    if body_start_offset.is_none() {
        return Vec::new();
    }

    let (keyword_line, _) = source.offset_to_line_col(effective_opening);
    let (opening_line, _) = source.offset_to_line_col(opening_offset);
    let (closing_line, _) = source.offset_to_line_col(closing_offset);

    if keyword_line == closing_line {
        return Vec::new();
    }

    let Some(body_start_offset) = body_start_offset else {
        return Vec::new();
    };
    let (body_start_line, _) = source.offset_to_line_col(body_start_offset);

    // RuboCop still flags only the "beginning" offense for two-line blocks
    // where exactly one delimiter shares a body line, but only when the
    // effective opening line is the physical `do`/`{` line. When multiline
    // lambda params move the effective opening back to the `->` line, the
    // shared helper still matches RuboCop and may require an end offense too.
    // Two-line blocks where exactly one delimiter shares a body line only need
    // the "beginning" offense. This applies when the effective opening is the
    // physical `do`/`{` line — either body starts on the opening line
    // (`it { foo.\n  bar }`) or on the closing line (`items.each { |x|\n  puts x }`).
    // When multiline lambda params move the effective opening back to `->`,
    // the shared helper handles it and may require an end offense too.
    if keyword_line == opening_line
        && closing_line == opening_line + 1
        && (body_start_line == opening_line || body_start_line == closing_line)
    {
        let mut diagnostics = Vec::new();
        if let Some(diag) =
            missing_beginning_empty_line_diagnostic(cop_name, source, keyword_line + 1, corrections)
        {
            diagnostics.push(diag);
        }
        return diagnostics;
    }

    util::check_missing_empty_lines_around_body_with_corrections(
        cop_name,
        source,
        effective_opening,
        closing_offset,
        "block",
        corrections,
    )
}

impl Cop for EmptyLinesAroundBlockBody {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundBlockBody"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[BLOCK_NODE, LAMBDA_NODE, SUPER_NODE, FORWARDING_SUPER_NODE]
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
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "no_empty_lines");
        let (opening_offset, closing_offset, lambda_operator_offset, body_start_offset) =
            if let Some(b) = node.as_block_node() {
                (
                    b.opening_loc().start_offset(),
                    b.closing_loc().start_offset(),
                    None,
                    b.body().map(|body| body.location().start_offset()),
                )
            } else if let Some(l) = node.as_lambda_node() {
                (
                    l.opening_loc().start_offset(),
                    l.closing_loc().start_offset(),
                    Some(l.operator_loc().start_offset()),
                    l.body().map(|body| body.location().start_offset()),
                )
            } else if let Some(s) = node.as_super_node() {
                let Some(block) = s.block().and_then(|block| block.as_block_node()) else {
                    return;
                };
                (
                    block.opening_loc().start_offset(),
                    block.closing_loc().start_offset(),
                    None,
                    block.body().map(|body| body.location().start_offset()),
                )
            } else if let Some(s) = node.as_forwarding_super_node() {
                let Some(block) = s.block() else {
                    return;
                };
                (
                    block.opening_loc().start_offset(),
                    block.closing_loc().start_offset(),
                    None,
                    block.body().map(|body| body.location().start_offset()),
                )
            } else {
                return;
            };

        // For the "beginning" check, determine the effective opening line.
        //
        // RuboCop uses `send_node.last_line` as the reference:
        // - For regular blocks, `send_node` includes the method call args,
        //   so `last_line` is the line with `do`/`{` (or the last arg line).
        // - For lambda blocks, `send_node` is just `send(nil, :lambda)`
        //   (the `->` operator), so `last_line` is the `->` line — NOT the
        //   `do`/`{` line when params span multiple lines.
        //
        // When a lambda has multiline params (`-> (a,\n b) do`), the `->` is
        // on an earlier line than `do`/`{`. Using `->` as the reference means
        // the line after `->` is a param continuation, not blank, so no FP.
        //
        // For `no_empty_lines`, we walk ALL the way back through `\` so the
        // next line after the reference is a continuation string (never blank).
        // For `empty_lines`, we step back only one line (matching
        // `send_node.last_line`) so the `do` line is what gets checked.
        let full_walkback = style != "empty_lines";
        let effective_opening = if let Some(op_offset) = lambda_operator_offset {
            let (op_line, _) = source.offset_to_line_col(op_offset);
            let (opening_line, _) = source.offset_to_line_col(opening_offset);
            if op_line != opening_line {
                // Multiline lambda params: use the -> line as reference
                op_offset
            } else {
                // Single-line: -> and do/{ on same line, use normal logic
                adjusted_keyword_offset(source, opening_offset, full_walkback)
            }
        } else {
            // Regular block: walk backward through backslash continuations
            adjusted_keyword_offset(source, opening_offset, full_walkback)
        };

        match style {
            "empty_lines" => {
                diagnostics.extend(check_empty_lines_style_with_rubocop_edge_cases(
                    self.name(),
                    source,
                    effective_opening,
                    opening_offset,
                    closing_offset,
                    body_start_offset,
                    corrections,
                ));
            }
            _ => {
                diagnostics.extend(util::check_empty_lines_around_body_with_corrections(
                    self.name(),
                    source,
                    effective_opening,
                    closing_offset,
                    "block",
                    corrections,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        EmptyLinesAroundBlockBody,
        "cops/layout/empty_lines_around_block_body"
    );
    crate::cop_autocorrect_fixture_tests!(
        EmptyLinesAroundBlockBody,
        "cops/layout/empty_lines_around_block_body"
    );

    #[test]
    fn single_line_block_no_offense() {
        let src = b"[1, 2, 3].each { |x| puts x }\n";
        let diags = run_cop_full(&EmptyLinesAroundBlockBody, src);
        assert!(diags.is_empty(), "Single-line block should not trigger");
    }

    #[test]
    fn do_end_block_with_blank_lines() {
        let src = b"items.each do |x|\n\n  puts x\n\nend\n";
        let diags = run_cop_full(&EmptyLinesAroundBlockBody, src);
        assert_eq!(
            diags.len(),
            2,
            "Should flag both beginning and end blank lines"
        );
    }

    #[test]
    fn empty_lines_style_requires_blank_lines() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("empty_lines".into()),
            )]),
            ..CopConfig::default()
        };
        // Block WITHOUT blank lines at beginning/end
        let src = b"items.each do |x|\n  puts x\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundBlockBody, src, config);
        assert_eq!(
            diags.len(),
            2,
            "empty_lines style should require blank lines at both ends"
        );
    }

    #[test]
    fn empty_lines_style_accepts_blank_lines() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("empty_lines".into()),
            )]),
            ..CopConfig::default()
        };
        // Block WITH blank lines at beginning/end
        let src = b"items.each do |x|\n\n  puts x\n\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundBlockBody, src, config);
        assert!(
            diags.is_empty(),
            "empty_lines style should accept blank lines"
        );
    }

    #[test]
    fn lambda_multiline_params_blank_after_do_no_offense() {
        // RuboCop uses send_node.last_line (the -> line) as the reference,
        // so the blank line after `do` is not adjacent to the opening.
        let src = b"f = -> (a:,\n        b:) do\n\n  something\nend\n";
        let diags = run_cop_full(&EmptyLinesAroundBlockBody, src);
        assert!(
            diags.is_empty(),
            "Lambda with multiline params should not flag blank line after do"
        );
    }

    #[test]
    fn lambda_single_line_params_blank_after_do_offense() {
        // When -> and do are on the same line, blank line after do IS flagged.
        let src = b"f = -> (a) do\n\n  something\nend\n";
        let diags = run_cop_full(&EmptyLinesAroundBlockBody, src);
        assert_eq!(
            diags.len(),
            1,
            "Lambda with single-line params should flag blank line after do"
        );
    }

    #[test]
    fn empty_lines_multiline_lambda_body_on_opening_line_still_requires_end() {
        use crate::testutil::run_cop_full_with_config;

        let src = b"handler = -> (first:,\n              second:) { do_something.\n  call_it }\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundBlockBody, src, empty_lines_config());

        assert_eq!(
            diags.len(),
            2,
            "multiline lambda params should still require both beginning and end offenses"
        );
        assert!(diags.iter().any(|d| {
            d.location.line == 2 && d.message == "Empty line missing at block body beginning."
        }));
        assert!(diags.iter().any(|d| {
            d.location.line == 3 && d.message == "Empty line missing at block body end."
        }));
    }

    #[test]
    fn empty_lines_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &EmptyLinesAroundBlockBody,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/empty_lines_around_block_body/empty_lines_offense.rb"
            ),
            empty_lines_config(),
        );
    }

    #[test]
    fn empty_lines_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &EmptyLinesAroundBlockBody,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/empty_lines_around_block_body/empty_lines_no_offense.rb"
            ),
            empty_lines_config(),
        );
    }

    fn empty_lines_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("empty_lines".into()),
            )]),
            ..CopConfig::default()
        }
    }
}
