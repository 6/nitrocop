use std::collections::HashSet;

use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Layout/SpaceAroundOperators checks that operators have space around them.
///
/// Investigation findings (2026-03-15):
/// - Original implementation had 190 FPs and 4,362 FNs.
/// - The massive FN count came from missing AST-based detection for:
///   compound assignments (+=, -=, *=, ||=, &&=, etc.), match operators (=~, !~),
///   class inheritance (<), singleton class (<<), rescue =>, === operator,
///   setter methods (x.y = 2), and exponent ** with spaces (no_space default).
/// - FPs came from the text scanner incorrectly flagging edge cases.
/// - Fix: expanded AST visitor to cover all operator types that RuboCop checks,
///   including write nodes (assignments), class/sclass operators, rescue assoc,
///   pattern matching operators (alternation |, capture =>), and rational literals.
///
/// Investigation findings (2026-03-18):
/// - FP=317: 109 from text scanner not treating tabs as valid whitespace around
///   operators (==, !=, =>, =). 205 from AllowForAlignment not supporting
///   cross-operator alignment (e.g., `||=` aligned with `=`). 3 from rational
///   literal false positives.
/// - FN=3040: 1492 missing extra-space detection for `=`, 1250 for `=>`,
///   114 for `==`, 83 for ternary `?`/`:` (not implemented).
/// - Fix: treat tabs as valid whitespace in text scanner; add extra-space
///   detection for `=` and `=>` in text scanner; improve alignment detection
///   to support cross-operator alignment (operators ending at same column).
///
/// ## Corpus investigation (2026-03-24)
///
/// Corpus oracle reported FP=1,492, FN=861.
///
/// An attempt was made to fix alignment detection with: (1) code-map awareness
/// for alignment checks, (2) word-boundary restriction, (3) assignment-group
/// alignment for `=`. The approach (commit 884f8c2, reverted) fixed FPs but
/// massively increased FNs from 861 to 12,736 — the assignment-group alignment
/// logic was too aggressively suppressing offenses. The code-map and word-boundary
/// changes also suppressed too many legitimate detections.
///
/// A correct fix needs to: implement assignment-group alignment narrowly (only
/// for consecutive simple `=` assignments at the same indentation level, not
/// for `==`, `!=`, `=>`, etc.), and ensure code-map checks don't suppress
/// operators that happen to be adjacent to string literals.
///
/// ## Corpus investigation (2026-04-01, run 23848125495, timed out)
///
/// Attempted fix: collect plain assignment `=` offsets from Prism
/// (LocalVariableWriteNode, InstanceVariableWriteNode, ClassVariableWriteNode,
/// GlobalVariableWriteNode, MultiWriteNode) and apply the assignment-specific
/// leading-space rule only to those offsets, keeping setter/index writes and
/// extra trailing spaces unchanged. Also added ternary `?`/`:` detection via
/// `visit_if_node` (checking `if_keyword_loc().is_none()` for ternary form).
///
/// Result: removed 21 FPs but introduced 16 new FPs (all in ruby__tk repo).
/// Net -5 FP with tests still failing at timeout. The alignment-block-neighbor
/// detection (`assignment_block_neighbor_line`) was too loose — it walked up/down
/// from the current line looking for lines at the same indentation with
/// assignment operators, but didn't correctly handle cases where a later `==` on
/// the same line aligned with the `=`, causing real offenses to be suppressed.
///
/// Key findings:
/// - RuboCop accepts extra leading spaces before plain `=` when the write is
///   standalone or part of a same-indentation assignment group whose `=` tokens
///   align with the *first* alignment operator on neighbor lines.
/// - The `first_alignment_operator_end_col()` approach is correct for finding
///   the alignment anchor, but the neighbor-line search needs tighter scoping.
/// - Ternary `?`/`:` via `visit_if_node` works but was not the cause of test
///   failures.
///
/// Investigation findings (2026-04-03):
/// - RuboCop allows extra spaces *after* `=` when the right-hand sides are
///   vertically aligned for plain assignments, e.g. `email =  "foo"` next to
///   `password = "bar"`.
/// - RuboCop does **not** use that same RHS rule for hash rockets: `=>  ` in a
///   multi-line hash pair is accepted when the pairs themselves align, even if
///   the value expressions start in different columns.
/// - RuboCop also still flags the same `=  ` padding on setter/index writes
///   such as `message[:bcc] =           'x'`; the RHS-alignment exception is
///   specific to plain assignment nodes.
/// - The previous implementation reused operator-column alignment for both
///   leading and trailing space checks, which caused false positives for
///   aligned RHS values and false negatives for `=  {` / `=  [` because the
///   aligned `=` token incorrectly suppressed the offense.
/// - Fix: keep operator alignment for extra leading space, use RHS-start
///   alignment for assignment-like trailing space, and use pair-start
///   alignment for `=>` inside `AssocNode`.
///
/// ## Corpus fix (2026-04-04)
///
/// FP=2,276 → ~1,794 (-482), FN=2,280 → ~2,134 (-146).
///
/// Two root causes identified and fixed:
///
/// 1. **Word boundary check too loose for leading space alignment** (FN fix):
///    `check_alignment_standalone` had a "word/space boundary" check (check 2)
///    that accepted any word boundary at the same column as alignment. This
///    incorrectly suppressed detection when e.g. `<<` at col 10 aligned with
///    `=` at col 10 on a neighbor line. RuboCop's `aligned_operator?` does NOT
///    use word boundary for leading-space alignment — it only uses identical
///    operator or cross-operator end-column alignment. Removed check 2.
///
/// 2. **Plain `=` assignments flagged without subsequent neighbor** (FP fix):
///    RuboCop's `excess_leading_space?` for `:assignment` type only flags
///    when a subsequent assignment at the same indent exists but is NOT
///    aligned. If no subsequent assignment exists (standalone or end-of-group),
///    RuboCop returns false (no offense). Added
///    `has_subsequent_assignment_neighbor` to replicate this behavior for
///    plain `=` assignments in the text scanner.
///
/// ## Corpus fix (2026-04-05)
///
/// Added ternary `?`/`:` operator detection via `visit_if_node`. In Prism,
/// ternary `cond ? expr : expr` is an IfNode with `if_keyword_loc() == None`.
/// The `?` location comes from `then_keyword_loc()`, and the `:` location
/// from the subsequent ElseNode's `else_keyword_loc()`.
///
/// Corpus FN analysis showed ternary `?` (33%) and `:` (29%) account for
/// ~62% of all FN examples sampled. Quick check (5 repos): resolved 341 FN
/// and 15 FP with 0 new FP and 0 new FN.
///
/// Remaining FN categories: extra-space `=` on aligned assignments (~15%),
/// extra-space ternary `?`/`:` (~15%), extra-space `=>` (~3%),
/// keyword `and`/`or` (~3%).
///
/// Remaining FP: setter/index `=  ` trailing space where RHS is aligned with
/// a plain assignment on a neighbor line across method boundaries. RuboCop's
/// `excess_trailing_space?` uses `aligned_with_something?` which checks for
/// word/space boundaries on adjacent lines — this is more permissive than our
/// `is_aligned_rhs_standalone`.
pub struct SpaceAroundOperators;

/// Collect byte offsets of `=` signs that are part of parameter defaults,
/// and byte ranges of operator method names in `def` statements.
struct ExclusionCollector {
    /// Byte offsets of `=` in default parameter positions.
    default_param_offsets: HashSet<usize>,
    /// Byte offsets of plain assignment `=` operators where RuboCop allows
    /// aligned RHS spacing.
    plain_assignment_offsets: HashSet<usize>,
    /// Byte ranges (start..end) of operator method names in `def` statements.
    /// e.g., `def ==(other)` — the `==` is a method name, not an operator.
    def_method_name_ranges: Vec<std::ops::Range<usize>>,
}

impl<'pr> Visit<'pr> for ExclusionCollector {
    fn visit_optional_parameter_node(&mut self, node: &ruby_prism::OptionalParameterNode<'pr>) {
        let op_loc = node.operator_loc();
        self.default_param_offsets.insert(op_loc.start_offset());
    }

    fn visit_optional_keyword_parameter_node(
        &mut self,
        _node: &ruby_prism::OptionalKeywordParameterNode<'pr>,
    ) {
        // Keyword params use `:` not `=`, so nothing to exclude.
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let name = node.name().as_slice();
        // Check if the method name contains operator characters that this cop checks
        let is_operator_name = name.contains(&b'=')
            || name.contains(&b'!')
            || name.contains(&b'>')
            || name.contains(&b'<')
            || name.contains(&b'+')
            || name.contains(&b'-')
            || name.contains(&b'*')
            || name.contains(&b'/')
            || name.contains(&b'%')
            || name.contains(&b'&')
            || name.contains(&b'|')
            || name.contains(&b'^')
            || name.contains(&b'~');
        if is_operator_name {
            let loc = node.name_loc();
            self.def_method_name_ranges
                .push(loc.start_offset()..loc.end_offset());
        }
        // Recurse into the body to find nested defs and default params
        ruby_prism::visit_def_node(self, node);
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        self.plain_assignment_offsets
            .insert(node.operator_loc().start_offset());
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_instance_variable_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableWriteNode<'pr>,
    ) {
        self.plain_assignment_offsets
            .insert(node.operator_loc().start_offset());
        ruby_prism::visit_instance_variable_write_node(self, node);
    }

    fn visit_class_variable_write_node(&mut self, node: &ruby_prism::ClassVariableWriteNode<'pr>) {
        self.plain_assignment_offsets
            .insert(node.operator_loc().start_offset());
        ruby_prism::visit_class_variable_write_node(self, node);
    }

    fn visit_global_variable_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableWriteNode<'pr>,
    ) {
        self.plain_assignment_offsets
            .insert(node.operator_loc().start_offset());
        ruby_prism::visit_global_variable_write_node(self, node);
    }

    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        self.plain_assignment_offsets
            .insert(node.operator_loc().start_offset());
        ruby_prism::visit_constant_write_node(self, node);
    }

    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'pr>) {
        self.plain_assignment_offsets
            .insert(node.operator_loc().start_offset());
        ruby_prism::visit_constant_path_write_node(self, node);
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        self.plain_assignment_offsets
            .insert(node.operator_loc().start_offset());
        ruby_prism::visit_multi_write_node(self, node);
    }
}

impl Cop for SpaceAroundOperators {
    fn name(&self) -> &'static str {
        "Layout/SpaceAroundOperators"
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let allow_for_alignment = config.get_bool("AllowForAlignment", true);
        let enforced_style_exponent =
            config.get_str("EnforcedStyleForExponentOperator", "no_space");
        let enforced_style_rational =
            config.get_str("EnforcedStyleForRationalLiterals", "no_space");

        // Collect default parameter `=` offsets and operator method name ranges
        let mut collector = ExclusionCollector {
            default_param_offsets: HashSet::new(),
            plain_assignment_offsets: HashSet::new(),
            def_method_name_ranges: Vec::new(),
        };
        collector.visit(&parse_result.node());
        let default_param_offsets = collector.default_param_offsets;
        let plain_assignment_offsets = collector.plain_assignment_offsets;
        let def_name_ranges = collector.def_method_name_ranges;

        let exponent_no_space = enforced_style_exponent == "no_space";
        let rational_no_space = enforced_style_rational == "no_space";

        // AST-based check for binary operators, assignments, and other operator nodes.
        let mut op_checker = OperatorChecker {
            cop: self,
            source,
            diagnostics: Vec::new(),
            corrections: Vec::new(),
            has_corrections: corrections.is_some(),
            exponent_no_space,
            rational_no_space,
            allow_for_alignment,
            reported_offsets: HashSet::new(),
        };
        op_checker.visit(&parse_result.node());
        let reported_offsets = op_checker.reported_offsets.clone();
        diagnostics.extend(op_checker.diagnostics);
        if let Some(ref mut corr) = corrections {
            corr.extend(op_checker.corrections);
        }

        let bytes = source.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        // Helper closure: check if offset `pos` falls within any operator method name range
        let in_def_name = |pos: usize| -> bool { def_name_ranges.iter().any(|r| r.contains(&pos)) };

        while i < len {
            if !code_map.is_code(i) {
                i += 1;
                continue;
            }

            // Check for multi-char operators first: ==, !=, =>
            if i + 1 < len && code_map.is_code(i + 1) {
                let two = &bytes[i..i + 2];
                if two == b"==" || two == b"!=" || two == b"=>" {
                    // Skip if already reported by AST visitor
                    if reported_offsets.contains(&i) {
                        i += 2;
                        continue;
                    }
                    // Skip ===
                    if two == b"==" && i + 2 < len && bytes[i + 2] == b'=' {
                        i += 3;
                        continue;
                    }

                    // Skip `=>` that is part of `<=>` (spaceship operator):
                    // if byte at i is `=` and i-1 is `<`, this is `<=>` not `=>`
                    if two == b"=>" && i > 0 && bytes[i - 1] == b'<' {
                        i += 2;
                        continue;
                    }

                    // Skip operator method names: `def ==(other)`, `def !=(other)`
                    if in_def_name(i) {
                        i += 2;
                        continue;
                    }

                    // Skip method calls via `.` or `&.`: e.g., `x&.!= y`, `x.== y`
                    if i > 0 && bytes[i - 1] == b'.' {
                        i += 2;
                        continue;
                    }

                    let op_str = std::str::from_utf8(two).unwrap_or("??");
                    let space_before = i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t');
                    let space_after =
                        i + 2 < len && (bytes[i + 2] == b' ' || bytes[i + 2] == b'\t');
                    let newline_after =
                        i + 2 >= len || bytes[i + 2] == b'\n' || bytes[i + 2] == b'\r';
                    if !space_before || (!space_after && !newline_after) {
                        let (line, column) = source.offset_to_line_col(i);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            format!("Surrounding space missing for operator `{op_str}`."),
                        );
                        if let Some(ref mut corr) = corrections {
                            if !space_before {
                                corr.push(crate::correction::Correction {
                                    start: i,
                                    end: i,
                                    replacement: " ".to_string(),
                                    cop_name: self.name(),
                                    cop_index: 0,
                                });
                            }
                            if !space_after && !newline_after {
                                corr.push(crate::correction::Correction {
                                    start: i + 2,
                                    end: i + 2,
                                    replacement: " ".to_string(),
                                    cop_name: self.name(),
                                    cop_index: 0,
                                });
                            }
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    } else if allow_for_alignment && space_before && (space_after || newline_after)
                    {
                        // Check for extra spaces around operator (alignment check)
                        let multi_before = i >= 2 && bytes[i - 1] == b' ' && bytes[i - 2] == b' ';
                        let multi_after =
                            i + 3 < len && bytes[i + 2] == b' ' && bytes[i + 3] == b' ';
                        if multi_before || multi_after {
                            check_text_scanner_extra_space(
                                self,
                                source,
                                i,
                                i + 2,
                                op_str,
                                two,
                                multi_before,
                                multi_after,
                                false,
                                code_map,
                                diagnostics,
                                &mut corrections,
                            );
                        }
                    }
                    i += 2;
                    continue;
                }
            }

            // Single = (not ==, !=, =>, =~, <=, >=, or part of +=/-=/etc.)
            if bytes[i] == b'=' {
                // Skip if already reported by AST visitor
                if reported_offsets.contains(&i) {
                    i += 1;
                    continue;
                }
                // Skip =~ and =>
                if i + 1 < len && (bytes[i + 1] == b'~' || bytes[i + 1] == b'>') {
                    i += 2;
                    continue;
                }
                // Skip ==
                if i + 1 < len && bytes[i + 1] == b'=' {
                    i += 2;
                    continue;
                }
                // Skip if preceded by !, <, >, =, +, -, *, /, %, &, |, ^, ~
                if i > 0 {
                    let prev = bytes[i - 1];
                    if matches!(
                        prev,
                        b'!' | b'<'
                            | b'>'
                            | b'='
                            | b'+'
                            | b'-'
                            | b'*'
                            | b'/'
                            | b'%'
                            | b'&'
                            | b'|'
                            | b'^'
                            | b'~'
                    ) {
                        i += 1;
                        continue;
                    }
                }

                // Skip default parameter `=` signs (handled by SpaceAroundEqualsInParameterDefault)
                if default_param_offsets.contains(&i) {
                    i += 1;
                    continue;
                }

                // Skip `=` that is part of an operator method name: `def []=`, `def ===`
                if in_def_name(i) {
                    i += 1;
                    continue;
                }

                let space_before = i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t');
                let space_after = i + 1 < len && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t');
                let newline_after = i + 1 >= len || bytes[i + 1] == b'\n' || bytes[i + 1] == b'\r';
                if !space_before || (!space_after && !newline_after) {
                    let (line, column) = source.offset_to_line_col(i);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Surrounding space missing for operator `=`.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        if !space_before {
                            corr.push(crate::correction::Correction {
                                start: i,
                                end: i,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                        }
                        if !space_after && !newline_after {
                            corr.push(crate::correction::Correction {
                                start: i + 1,
                                end: i + 1,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                        }
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                } else if allow_for_alignment && space_before && (space_after || newline_after) {
                    // Check for extra spaces around `=` (alignment check)
                    let multi_before = i >= 2 && bytes[i - 1] == b' ' && bytes[i - 2] == b' ';
                    let multi_after = i + 2 < len && bytes[i + 1] == b' ' && bytes[i + 2] == b' ';
                    if multi_before || multi_after {
                        check_text_scanner_extra_space(
                            self,
                            source,
                            i,
                            i + 1,
                            "=",
                            b"=",
                            multi_before,
                            multi_after,
                            plain_assignment_offsets.contains(&i),
                            code_map,
                            diagnostics,
                            &mut corrections,
                        );
                    }
                }
                i += 1;
                continue;
            }

            i += 1;
        }
    }
}

/// Check for extra spaces around an operator found by the text scanner.
#[allow(clippy::too_many_arguments)]
fn check_text_scanner_extra_space(
    cop: &SpaceAroundOperators,
    source: &SourceFile,
    op_start: usize,
    op_end: usize,
    op_str: &str,
    op_bytes: &[u8],
    multi_before: bool,
    multi_after: bool,
    is_plain_assignment: bool,
    code_map: &CodeMap,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
) {
    let bytes = source.as_bytes();
    let mut multi_before = multi_before;
    let mut multi_after = multi_after;

    // Skip if operator is at start of line (spaces are indentation)
    if multi_before {
        let mut ls = op_start;
        while ls > 0 && bytes[ls - 1] != b'\n' {
            ls -= 1;
        }
        if bytes[ls..op_start].iter().all(|&b| b == b' ' || b == b'\t') {
            multi_before = false;
        }
    }

    if multi_before && is_aligned_standalone(source, op_start, op_bytes) {
        multi_before = false;
    }

    // RuboCop-compatible: for plain assignments (=), only flag extra leading space
    // if there IS a subsequent assignment neighbor at the same indentation that is
    // NOT aligned. If there's no subsequent assignment, it's standalone or end-of-group
    // and should not be flagged.
    if multi_before
        && is_plain_assignment
        && !has_subsequent_assignment_neighbor(source, op_start, code_map)
    {
        multi_before = false;
    }

    if multi_after {
        let mut p = op_end;
        while p < bytes.len() && bytes[p] == b' ' {
            p += 1;
        }
        if p < bytes.len() && bytes[p] == b'#' {
            multi_after = false;
        } else if is_plain_assignment || op_bytes != b"=" {
            if let Some(rhs_start) = util::first_non_space_on_line(bytes, op_end) {
                if is_aligned_rhs_standalone(source, rhs_start) {
                    multi_after = false;
                }
            }
        }
    }

    if !multi_before && !multi_after {
        return;
    }

    let ws_start = if multi_before {
        let mut s = op_start - 1;
        while s > 0 && bytes[s - 1] == b' ' {
            s -= 1;
        }
        s
    } else {
        op_start
    };
    let ws_end = if multi_after {
        let mut e = op_end;
        while e < bytes.len() && bytes[e] == b' ' {
            e += 1;
        }
        e
    } else {
        op_end
    };
    let (line, column) = source.offset_to_line_col(op_start);
    let mut diag = cop.diagnostic(
        source,
        line,
        column,
        format!("Operator `{op_str}` should be surrounded by a single space."),
    );
    if let Some(corr) = corrections {
        if multi_before {
            corr.push(crate::correction::Correction {
                start: ws_start,
                end: op_start,
                replacement: " ".to_string(),
                cop_name: cop.name(),
                cop_index: 0,
            });
        }
        if multi_after {
            corr.push(crate::correction::Correction {
                start: op_end,
                end: ws_end,
                replacement: " ".to_string(),
                cop_name: cop.name(),
                cop_index: 0,
            });
        }
        diag.corrected = true;
    }
    diagnostics.push(diag);
}

/// Count UTF-8 codepoints from the start of `line` up to `byte_col` bytes.
/// For ASCII-only lines this equals `byte_col`; for lines with multi-byte chars
/// (e.g. curly quotes) it returns the visual character column.
fn bytes_to_char_col(line: &[u8], byte_col: usize) -> usize {
    let capped = byte_col.min(line.len());
    let mut chars = 0usize;
    let mut i = 0usize;
    while i < capped {
        let b = line[i];
        let width = if b < 0x80 {
            1
        } else if b & 0xE0 == 0xC0 {
            2
        } else if b & 0xF0 == 0xE0 {
            3
        } else {
            4
        };
        i += width;
        chars += 1;
    }
    chars
}

/// Return the byte offset within `line` that starts character column `char_col`.
/// Returns `None` if the line is shorter than `char_col` characters.
fn char_col_to_bytes(line: &[u8], char_col: usize) -> Option<usize> {
    let mut chars = 0usize;
    let mut i = 0usize;
    while i < line.len() {
        if chars == char_col {
            return Some(i);
        }
        let b = line[i];
        let width = if b < 0x80 {
            1
        } else if b & 0xE0 == 0xC0 {
            2
        } else if b & 0xF0 == 0xE0 {
            3
        } else {
            4
        };
        i += width;
        chars += 1;
    }
    if chars == char_col { Some(i) } else { None }
}

/// Check if the operator at byte offset `start` is aligned with an operator
/// on an adjacent non-blank, non-comment line. Supports:
///
/// 1. Same operator at same char column
/// 2. Word/space boundary at same column (aligned_words in RuboCop)
/// 3. Cross-operator alignment (operators ending at same column)
fn is_aligned_standalone(source: &SourceFile, start: usize, op_bytes: &[u8]) -> bool {
    let bytes = source.as_bytes();
    let mut ls = start;
    while ls > 0 && bytes[ls - 1] != b'\n' {
        ls -= 1;
    }
    let byte_col = start - ls;
    let lines: Vec<&[u8]> = source.lines().collect();
    let (line, _) = source.offset_to_line_col(start);
    let line_idx = line - 1;
    // Use character column so that multi-byte UTF-8 chars (e.g. curly quotes)
    // before the operator don't break alignment detection on adjacent ASCII lines.
    let char_col = bytes_to_char_col(lines[line_idx], byte_col);
    // All alignment operators are ASCII, so char length == byte length.
    let char_end_col = char_col + op_bytes.len();
    // Pass 1: closest non-blank, non-comment line (no indentation filter)
    if check_alignment_standalone(&lines, line_idx, char_col, char_end_col, op_bytes, None) {
        return true;
    }
    // Pass 2: search for same-indentation lines further out
    let my_indent = lines[line_idx]
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(0);
    check_alignment_standalone(
        &lines,
        line_idx,
        char_col,
        char_end_col,
        op_bytes,
        Some(my_indent),
    )
}

fn check_alignment_standalone(
    lines: &[&[u8]],
    line_idx: usize,
    char_col: usize,
    char_end_col: usize,
    op_bytes: &[u8],
    indent_filter: Option<usize>,
) -> bool {
    for up in [true, false] {
        let mut check_idx = if up {
            if line_idx == 0 {
                continue;
            }
            line_idx - 1
        } else {
            line_idx + 1
        };
        loop {
            if check_idx >= lines.len() {
                break;
            }
            let line_bytes = lines[check_idx];
            let first_non_ws = line_bytes.iter().position(|&b| b != b' ' && b != b'\t');
            match first_non_ws {
                None => {}                               // Empty line — skip
                Some(fs) if line_bytes[fs] == b'#' => {} // Comment line — skip
                Some(indent) => {
                    if let Some(required) = indent_filter {
                        if indent != required {
                            if up {
                                if check_idx == 0 {
                                    break;
                                }
                                check_idx -= 1;
                            } else {
                                check_idx += 1;
                            }
                            continue;
                        }
                    }
                    // Convert char_col back to byte offset for this specific line.
                    // This handles lines where multi-byte chars (e.g. curly-quote string
                    // keys) appear before the operator, shifting the byte offset.
                    if let Some(byte_col) = char_col_to_bytes(line_bytes, char_col) {
                        // Check 1: same operator at same char column
                        if byte_col + op_bytes.len() <= line_bytes.len()
                            && &line_bytes[byte_col..byte_col + op_bytes.len()] == op_bytes
                        {
                            return true;
                        }
                    }
                    // Check 3: cross-operator alignment (operators ending at same char column)
                    if line_has_operator_ending_at_char_col(line_bytes, char_end_col) {
                        return true;
                    }
                    break;
                }
            }
            if up {
                if check_idx == 0 {
                    break;
                }
                check_idx -= 1;
            } else {
                check_idx += 1;
            }
        }
    }
    false
}

/// Check if a line has an assignment/comparison operator ending at the given
/// *character* column (codepoint index, not byte index).
fn line_has_operator_ending_at_char_col(line: &[u8], target_char_end_col: usize) -> bool {
    let Some(target_end_col) = char_col_to_bytes(line, target_char_end_col) else {
        return false;
    };
    line_has_operator_ending_at_col(line, target_end_col)
}

/// Check if a line has an assignment/comparison operator ending at the given
/// *byte* column. This enables cross-operator alignment detection,
/// e.g., `=` aligned with `||=`.
fn line_has_operator_ending_at_col(line: &[u8], target_end_col: usize) -> bool {
    if target_end_col == 0 || target_end_col > line.len() {
        return false;
    }
    let end_byte = line[target_end_col - 1];
    if end_byte != b'=' && end_byte != b'<' {
        return false;
    }
    let col = target_end_col;
    if end_byte == b'=' && col >= 1 {
        // Skip if this `=` is actually the start of a multi-char operator (=>, ==, =~)
        if col < line.len() && matches!(line[col], b'>' | b'=' | b'~') {
            return false;
        }
        let before = if col >= 2 { line[col - 2] } else { b' ' };
        // Simple `=` preceded by whitespace
        if before == b' ' || before == b'\t' {
            return true;
        }
        // `==`, `!=`, `<=`, `>=`
        if matches!(before, b'=' | b'!' | b'<' | b'>') {
            return true;
        }
        // `+=`, `-=`, `*=`, `/=`, `%=`, `^=`, `|=`, `&=`
        if matches!(
            before,
            b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'|' | b'&'
        ) {
            return true;
        }
        // `||=`, `&&=`, `**=`, `<<=`, `>>=`
        if col >= 3 {
            let two_before = &line[col - 3..col];
            if two_before == b"||="
                || two_before == b"&&="
                || two_before == b"**="
                || two_before == b"<<="
                || two_before == b">>="
            {
                return true;
            }
        }
    }
    // `<<` (append operator, treated as assignment-like for alignment)
    if end_byte == b'<' && col >= 2 && line[col - 2] == b'<' {
        return true;
    }
    false
}

/// Check if there is a subsequent (below) assignment-like line at the same
/// indentation level. This mirrors RuboCop's `aligned_with_subsequent_equals_operator`
/// which returns `:none` (suppressing the offense) when no subsequent assignment exists.
///
/// Uses code_map to avoid matching `=` inside strings or comments.
fn has_subsequent_assignment_neighbor(
    source: &SourceFile,
    op_start: usize,
    code_map: &CodeMap,
) -> bool {
    let bytes = source.as_bytes();
    let lines: Vec<&[u8]> = source.lines().collect();
    let (line, _) = source.offset_to_line_col(op_start);
    let line_idx = line - 1;

    let my_indent = lines[line_idx]
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(0);

    // Compute line start offsets for absolute position mapping
    let line_starts = compute_line_starts(bytes);

    // Scan downward from the line after current
    let mut check_idx = line_idx + 1;
    loop {
        if check_idx >= lines.len() {
            break;
        }
        let line_bytes = lines[check_idx];
        let first_non_ws = line_bytes.iter().position(|&b| b != b' ' && b != b'\t');
        match first_non_ws {
            None => {
                // Blank line: in RuboCop, a blank line after a relevant-indent line
                // terminates the search
                break;
            }
            Some(fs) if line_bytes[fs] == b'#' => {
                // Comment line — skip
                check_idx += 1;
                continue;
            }
            Some(indent) => {
                if indent < my_indent {
                    // Dedented line — stop search
                    break;
                }
                if indent == my_indent {
                    // Same indent — check if this line has an assignment operator in code
                    let abs_start = line_starts[check_idx];
                    return line_has_equals_sign_in_code(line_bytes, abs_start, code_map);
                }
                // Different indent (more indented) — skip (e.g., continuation)
                check_idx += 1;
                continue;
            }
        }
    }
    false
}

/// Compute the byte offset of each line's start within the source.
fn compute_line_starts(bytes: &[u8]) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Check if a line contains an `=` sign that looks like an assignment or
/// comparison operator, using the code_map to ensure the `=` is in code
/// (not inside a string, comment, or regexp).
fn line_has_equals_sign_in_code(line: &[u8], line_abs_start: usize, code_map: &CodeMap) -> bool {
    for i in 0..line.len() {
        if line[i] == b'=' {
            let abs_offset = line_abs_start + i;
            if !code_map.is_code(abs_offset) {
                continue;
            }
            // Skip `=` that is part of `=>` (hash rocket) — we only care about
            // assignment-like `=`
            if i + 1 < line.len() && line[i + 1] == b'>' {
                continue;
            }
            // Check that `=` is preceded by a valid operator/word character
            // (not a `!`, `<`, `>`, or another `=` which would make it `!=`, `<=`, `>=`, `==`)
            if i > 0 {
                let prev = line[i - 1];
                if prev == b' '
                    || prev == b'\t'
                    || prev.is_ascii_alphanumeric()
                    || prev == b'_'
                    || prev == b')'
                    || prev == b']'
                    || prev == b'|'
                    || prev == b'&'
                    || prev == b'*'
                    || prev == b'+'
                    || prev == b'-'
                    || prev == b'/'
                    || prev == b'%'
                    || prev == b'^'
                    || prev == b'<'
                    || prev == b'>'
                {
                    return true;
                }
            }
        }
    }
    false
}

fn is_aligned_rhs_standalone(source: &SourceFile, start: usize) -> bool {
    let bytes = source.as_bytes();
    let mut ls = start;
    while ls > 0 && bytes[ls - 1] != b'\n' {
        ls -= 1;
    }

    let byte_col = start - ls;
    let lines: Vec<&[u8]> = source.lines().collect();
    let (line, _) = source.offset_to_line_col(start);
    let line_idx = line - 1;
    let char_col = bytes_to_char_col(lines[line_idx], byte_col);

    if check_rhs_alignment_standalone(&lines, line_idx, char_col, None) {
        return true;
    }

    let my_indent = lines[line_idx]
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(0);
    check_rhs_alignment_standalone(&lines, line_idx, char_col, Some(my_indent))
}

fn check_rhs_alignment_standalone(
    lines: &[&[u8]],
    line_idx: usize,
    char_col: usize,
    indent_filter: Option<usize>,
) -> bool {
    for up in [true, false] {
        let mut check_idx = if up {
            if line_idx == 0 {
                continue;
            }
            line_idx - 1
        } else {
            line_idx + 1
        };

        loop {
            if check_idx >= lines.len() {
                break;
            }

            let line_bytes = lines[check_idx];
            let first_non_ws = line_bytes.iter().position(|&b| b != b' ' && b != b'\t');
            match first_non_ws {
                None => {}
                Some(fs) if line_bytes[fs] == b'#' => {}
                Some(indent) => {
                    if let Some(required) = indent_filter {
                        if indent != required {
                            if up {
                                if check_idx == 0 {
                                    break;
                                }
                                check_idx -= 1;
                            } else {
                                check_idx += 1;
                            }
                            continue;
                        }
                    }

                    if line_has_aligned_rhs_at_char_col(line_bytes, char_col) {
                        return true;
                    }
                    break;
                }
            }

            if up {
                if check_idx == 0 {
                    break;
                }
                check_idx -= 1;
            } else {
                check_idx += 1;
            }
        }
    }

    false
}

fn line_has_aligned_rhs_at_char_col(line: &[u8], target_char_col: usize) -> bool {
    let Some(byte_col) = char_col_to_bytes(line, target_char_col) else {
        return false;
    };

    byte_col > 0
        && byte_col < line.len()
        && (line[byte_col - 1] == b' ' || line[byte_col - 1] == b'\t')
        && line[byte_col] != b' '
        && line[byte_col] != b'\t'
}

const BINARY_OPERATORS: &[&[u8]] = &[
    b"+", b"-", b"*", b"/", b"%", b"**", b"&", b"|", b"^", b"<<", b">>", b"<", b">", b"<=", b">=",
    b"<=>",
];

/// Additional operators detected via CallNode (match operators, ===)
const MATCH_OPERATORS: &[&[u8]] = &[b"=~", b"!~", b"==="];

struct OperatorChecker<'a> {
    cop: &'a SpaceAroundOperators,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    corrections: Vec<crate::correction::Correction>,
    has_corrections: bool,
    exponent_no_space: bool,
    rational_no_space: bool,
    allow_for_alignment: bool,
    /// Track byte offsets where offenses have been reported to avoid duplicates
    /// between the AST visitor and the text scanner.
    reported_offsets: HashSet<usize>,
}

impl OperatorChecker<'_> {
    /// Delegates to the standalone alignment checker which supports
    /// cross-operator alignment (e.g., `||=` aligned with `=`).
    fn is_aligned_with_adjacent(&self, start: usize, op_bytes: &[u8]) -> bool {
        is_aligned_standalone(self.source, start, op_bytes)
    }

    /// Check operator spacing for a "should have space" operator.
    /// Reports missing space or extra space around the operator.
    fn check_operator_spacing(&mut self, op_loc: &ruby_prism::Location<'_>) {
        self.check_operator_spacing_with_trailing_anchor(op_loc, None);
    }

    fn check_operator_spacing_with_trailing_anchor(
        &mut self,
        op_loc: &ruby_prism::Location<'_>,
        trailing_anchor: Option<usize>,
    ) {
        let start = op_loc.start_offset();
        let end = op_loc.end_offset();
        let bytes = self.source.as_bytes();
        let op_str = std::str::from_utf8(op_loc.as_slice()).unwrap_or("??");

        // Skip ** when exponent style is no_space — no-space offenses are handled by
        // check_no_space_operator instead.
        if op_str == "**" && self.exponent_no_space {
            return;
        }

        // Skip / for rational literals when rational style is no_space
        // (rational no-space offenses handled separately)
        if op_str == "/" && self.rational_no_space {
            // Check if the right operand is a rational literal (ends with 'r')
            if self.is_rational_division(end) {
                return;
            }
        }

        let has_space_before = start > 0 && (bytes[start - 1] == b' ' || bytes[start - 1] == b'\t');
        let has_space_after = end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'\t');
        let newline_after = end >= bytes.len() || bytes[end] == b'\n' || bytes[end] == b'\r';

        // Accept tabs as spacing (RuboCop: "accepts operator surrounded by tabs")
        if has_space_before && (has_space_after || newline_after) {
            // Check for multiple spaces (extra whitespace before or after operator)
            let multi_space_before =
                start >= 2 && bytes[start - 1] == b' ' && bytes[start - 2] == b' ';
            let multi_space_after =
                end + 1 < bytes.len() && bytes[end] == b' ' && bytes[end + 1] == b' ';

            if multi_space_before || multi_space_after {
                self.check_extra_space(start, end, op_str, op_loc.as_slice(), trailing_anchor);
            }
            return;
        }

        // Missing space — report offense
        if !has_space_before || (!has_space_after && !newline_after) {
            self.reported_offsets.insert(start);
            let (line, column) = self.source.offset_to_line_col(start);
            let mut diag = self.cop.diagnostic(
                self.source,
                line,
                column,
                format!("Surrounding space missing for operator `{op_str}`."),
            );
            if self.has_corrections {
                if !has_space_before {
                    self.corrections.push(crate::correction::Correction {
                        start,
                        end: start,
                        replacement: " ".to_string(),
                        cop_name: self.cop.name(),
                        cop_index: 0,
                    });
                }
                if !has_space_after && !newline_after {
                    self.corrections.push(crate::correction::Correction {
                        start: end,
                        end,
                        replacement: " ".to_string(),
                        cop_name: self.cop.name(),
                        cop_index: 0,
                    });
                }
                diag.corrected = true;
            }
            self.diagnostics.push(diag);
        }
    }

    /// Check for extra space around an operator (already has at least one space on each side).
    fn check_extra_space(
        &mut self,
        start: usize,
        end: usize,
        op_str: &str,
        op_bytes: &[u8],
        trailing_anchor: Option<usize>,
    ) {
        let bytes = self.source.as_bytes();
        let mut multi_space_before =
            start >= 2 && bytes[start - 1] == b' ' && bytes[start - 2] == b' ';
        let mut multi_space_after =
            end + 1 < bytes.len() && bytes[end] == b' ' && bytes[end + 1] == b' ';

        if !multi_space_before && !multi_space_after {
            return;
        }

        // Skip if operator is at start of line (spaces are indentation, not extra spacing)
        if multi_space_before {
            let mut ls = start;
            while ls > 0 && bytes[ls - 1] != b'\n' {
                ls -= 1;
            }
            if bytes[ls..start].iter().all(|&b| b == b' ' || b == b'\t') {
                multi_space_before = false;
            }
        }

        if self.allow_for_alignment
            && multi_space_before
            && self.is_aligned_with_adjacent(start, op_bytes)
        {
            multi_space_before = false;
        }

        if multi_space_after {
            let mut p = end;
            while p < bytes.len() && bytes[p] == b' ' {
                p += 1;
            }
            if p < bytes.len() && bytes[p] == b'#' {
                multi_space_after = false;
            } else if self.allow_for_alignment {
                if let Some(anchor) = trailing_anchor {
                    if is_aligned_rhs_standalone(self.source, anchor) {
                        multi_space_after = false;
                    }
                } else if let Some(rhs_start) = util::first_non_space_on_line(bytes, end) {
                    if is_aligned_rhs_standalone(self.source, rhs_start) {
                        multi_space_after = false;
                    }
                }
            }
        }

        if !multi_space_before && !multi_space_after {
            return;
        }

        // Find the extent of extra spaces before the operator
        let ws_start_before = if multi_space_before {
            let mut s = start - 1;
            while s > 0 && bytes[s - 1] == b' ' {
                s -= 1;
            }
            s
        } else {
            start
        };
        // Find the extent of extra spaces after the operator
        let ws_end_after = if multi_space_after {
            let mut e = end;
            while e < bytes.len() && bytes[e] == b' ' {
                e += 1;
            }
            e
        } else {
            end
        };
        self.reported_offsets.insert(start);
        let (line, column) = self.source.offset_to_line_col(start);
        let mut diag = self.cop.diagnostic(
            self.source,
            line,
            column,
            format!("Operator `{op_str}` should be surrounded by a single space."),
        );
        if self.has_corrections {
            if multi_space_before {
                self.corrections.push(crate::correction::Correction {
                    start: ws_start_before,
                    end: start,
                    replacement: " ".to_string(),
                    cop_name: self.cop.name(),
                    cop_index: 0,
                });
            }
            if multi_space_after {
                self.corrections.push(crate::correction::Correction {
                    start: end,
                    end: ws_end_after,
                    replacement: " ".to_string(),
                    cop_name: self.cop.name(),
                    cop_index: 0,
                });
            }
            diag.corrected = true;
        }
        self.diagnostics.push(diag);
    }

    /// Check operator that should NOT have surrounding space (e.g., ** with no_space style).
    /// Reports an offense if space IS present around the operator.
    fn check_no_space_operator(&mut self, op_loc: &ruby_prism::Location<'_>) {
        let start = op_loc.start_offset();
        let end = op_loc.end_offset();
        let bytes = self.source.as_bytes();
        let op_str = std::str::from_utf8(op_loc.as_slice()).unwrap_or("??");

        let space_before = start > 0 && bytes[start - 1] == b' ';
        let space_after = end < bytes.len() && bytes[end] == b' ';

        if space_before || space_after {
            self.reported_offsets.insert(start);
            let (line, column) = self.source.offset_to_line_col(start);
            let mut diag = self.cop.diagnostic(
                self.source,
                line,
                column,
                format!("Space around operator `{op_str}` detected."),
            );
            if self.has_corrections {
                // Remove space before
                if space_before {
                    let mut ws_start = start - 1;
                    while ws_start > 0 && bytes[ws_start - 1] == b' ' {
                        ws_start -= 1;
                    }
                    self.corrections.push(crate::correction::Correction {
                        start: ws_start,
                        end: start,
                        replacement: String::new(),
                        cop_name: self.cop.name(),
                        cop_index: 0,
                    });
                }
                // Remove space after
                if space_after {
                    let mut ws_end = end;
                    while ws_end < bytes.len() && bytes[ws_end] == b' ' {
                        ws_end += 1;
                    }
                    self.corrections.push(crate::correction::Correction {
                        start: end,
                        end: ws_end,
                        replacement: String::new(),
                        cop_name: self.cop.name(),
                        cop_index: 0,
                    });
                }
                diag.corrected = true;
            }
            self.diagnostics.push(diag);
        }
    }

    /// Check if the bytes after a `/` operator indicate a rational literal
    /// (a number immediately followed by 'r').
    fn is_rational_division(&self, slash_end: usize) -> bool {
        let bytes = self.source.as_bytes();
        let mut i = slash_end;
        // Skip spaces after /
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        // Look for digits followed by 'r'
        let digit_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i > digit_start && i < bytes.len() && bytes[i] == b'r' {
            return true;
        }
        false
    }
}

impl<'pr> Visit<'pr> for OperatorChecker<'_> {
    // === Binary operators via CallNode (including match operators and ===) ===
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let name = node.name().as_slice();

        // Check if this is a regular binary operator call (not via .method syntax)
        let is_operator = BINARY_OPERATORS.contains(&name) || MATCH_OPERATORS.contains(&name);
        if node.receiver().is_some()
            && node.call_operator_loc().is_none()
            && is_operator
            && (node.arguments().is_some() || MATCH_OPERATORS.contains(&name))
        {
            if let Some(msg_loc) = node.message_loc() {
                let op_bytes = msg_loc.as_slice();
                // Handle ** no_space and / rational no_space:
                // these operators should NOT have space around them
                let should_have_no_space = (op_bytes == b"**" && self.exponent_no_space)
                    || (op_bytes == b"/"
                        && self.rational_no_space
                        && self.is_rational_division(msg_loc.end_offset()));
                if should_have_no_space {
                    self.check_no_space_operator(&msg_loc);
                } else {
                    self.check_operator_spacing(&msg_loc);
                }
            }
        }

        ruby_prism::visit_call_node(self, node);
    }

    // === Logical operators (&&, ||) ===
    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        let op_loc = node.operator_loc();
        // Skip keyword form `and`
        if op_loc.as_slice() != b"and" {
            self.check_operator_spacing(&op_loc);
        }
        ruby_prism::visit_and_node(self, node);
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        let op_loc = node.operator_loc();
        // Skip keyword form `or`
        if op_loc.as_slice() != b"or" {
            self.check_operator_spacing(&op_loc);
        }
        ruby_prism::visit_or_node(self, node);
    }

    // === Compound assignment operators (+=, -=, *=, /=, %=, **=, <<=, >>=, ^=, |=, &=) ===
    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.binary_operator_loc());
        ruby_prism::visit_local_variable_operator_write_node(self, node);
    }

    fn visit_instance_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOperatorWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.binary_operator_loc());
        ruby_prism::visit_instance_variable_operator_write_node(self, node);
    }

    fn visit_class_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOperatorWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.binary_operator_loc());
        ruby_prism::visit_class_variable_operator_write_node(self, node);
    }

    fn visit_global_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOperatorWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.binary_operator_loc());
        ruby_prism::visit_global_variable_operator_write_node(self, node);
    }

    fn visit_constant_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantOperatorWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.binary_operator_loc());
        ruby_prism::visit_constant_operator_write_node(self, node);
    }

    fn visit_constant_path_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOperatorWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.binary_operator_loc());
        ruby_prism::visit_constant_path_operator_write_node(self, node);
    }

    fn visit_call_operator_write_node(&mut self, node: &ruby_prism::CallOperatorWriteNode<'pr>) {
        self.check_operator_spacing(&node.binary_operator_loc());
        ruby_prism::visit_call_operator_write_node(self, node);
    }

    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'pr>) {
        self.check_operator_spacing(&node.binary_operator_loc());
        ruby_prism::visit_index_operator_write_node(self, node);
    }

    // === ||= and &&= operators ===
    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_local_variable_or_write_node(self, node);
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_local_variable_and_write_node(self, node);
    }

    fn visit_instance_variable_or_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOrWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_instance_variable_or_write_node(self, node);
    }

    fn visit_instance_variable_and_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableAndWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_instance_variable_and_write_node(self, node);
    }

    fn visit_class_variable_or_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOrWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_class_variable_or_write_node(self, node);
    }

    fn visit_class_variable_and_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableAndWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_class_variable_and_write_node(self, node);
    }

    fn visit_global_variable_or_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOrWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_global_variable_or_write_node(self, node);
    }

    fn visit_global_variable_and_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableAndWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_global_variable_and_write_node(self, node);
    }

    fn visit_constant_or_write_node(&mut self, node: &ruby_prism::ConstantOrWriteNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_constant_or_write_node(self, node);
    }

    fn visit_constant_and_write_node(&mut self, node: &ruby_prism::ConstantAndWriteNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_constant_and_write_node(self, node);
    }

    fn visit_constant_path_or_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOrWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_constant_path_or_write_node(self, node);
    }

    fn visit_constant_path_and_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathAndWriteNode<'pr>,
    ) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_constant_path_and_write_node(self, node);
    }

    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_call_or_write_node(self, node);
    }

    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_call_and_write_node(self, node);
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_index_or_write_node(self, node);
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_index_and_write_node(self, node);
    }

    // === Class inheritance operator (<) ===
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        if let Some(op_loc) = node.inheritance_operator_loc() {
            self.check_operator_spacing(&op_loc);
        }
        ruby_prism::visit_class_node(self, node);
    }

    // === Singleton class operator (<<) ===
    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        let op_loc = node.operator_loc();
        self.check_operator_spacing(&op_loc);
        ruby_prism::visit_singleton_class_node(self, node);
    }

    // === Hash rocket operator (=>) ===
    fn visit_assoc_node(&mut self, node: &ruby_prism::AssocNode<'pr>) {
        if let Some(op_loc) = node.operator_loc() {
            if op_loc.as_slice() == b"=>" {
                // Mark pair `=>` as AST-covered so the text scanner does not
                // re-check it with the generic RHS rule.
                self.reported_offsets.insert(op_loc.start_offset());
                self.check_operator_spacing_with_trailing_anchor(
                    &op_loc,
                    Some(node.location().start_offset()),
                );
            }
        }
        ruby_prism::visit_assoc_node(self, node);
    }

    // === Rescue => operator ===
    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        if let Some(op_loc) = node.operator_loc() {
            self.check_operator_spacing(&op_loc);
        }
        ruby_prism::visit_rescue_node(self, node);
    }

    // === Pattern matching operators ===
    // `in pattern => var` (capture pattern)
    fn visit_capture_pattern_node(&mut self, node: &ruby_prism::CapturePatternNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_capture_pattern_node(self, node);
    }

    // `in pattern1 | pattern2` (alternation pattern)
    fn visit_alternation_pattern_node(&mut self, node: &ruby_prism::AlternationPatternNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_alternation_pattern_node(self, node);
    }

    // `expr => pattern` (match required, Ruby 3.0+)
    fn visit_match_required_node(&mut self, node: &ruby_prism::MatchRequiredNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_match_required_node(self, node);
    }

    // `expr in pattern` (match predicate) — uses keyword `in`, not checked here
    // (Layout/SpaceAroundKeyword handles `in`)

    // === Ternary operator (? and :) ===
    // `cond ? then_expr : else_expr`
    // In Prism, ternary is an IfNode with if_keyword_loc() == None.
    // The `?` location comes from then_keyword_loc().
    // The `:` location comes from the subsequent ElseNode's else_keyword_loc().
    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        let is_ternary = node.if_keyword_loc().is_none();
        if is_ternary {
            // Check spacing around `?`
            if let Some(q_loc) = node.then_keyword_loc() {
                if q_loc.as_slice() == b"?" {
                    self.check_operator_spacing(&q_loc);
                }
            }
            // Check spacing around `:`
            if let Some(sub) = node.subsequent() {
                if let Some(else_node) = sub.as_else_node() {
                    let colon_loc = else_node.else_keyword_loc();
                    if colon_loc.as_slice() == b":" {
                        self.check_operator_spacing(&colon_loc);
                    }
                }
            }
        }
        ruby_prism::visit_if_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(SpaceAroundOperators, "cops/layout/space_around_operators");
    crate::cop_autocorrect_fixture_tests!(
        SpaceAroundOperators,
        "cops/layout/space_around_operators"
    );
}
