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
///
/// ## Corpus fix (2026-04-05, attempt 16)
///
/// Enabled RHS alignment checking for setter `=` trailing space.  Previously
/// the trailing-space branch in `check_text_scanner_extra_space` only ran for
/// plain assignments (`is_plain_assignment`) or non-`=` operators, which meant
/// setter calls like `cors_rule.allowed_origins =  foo` were always flagged
/// even when the RHS values aligned across adjacent lines.
///
/// RuboCop's `excess_trailing_space?` applies `aligned_with_something?` to the
/// right operand for ALL operator types, including setter `=`.  Index writes
/// (`x[:key] = value`) are excluded because RuboCop checks alignment on the
/// key position inside brackets, not the value — we detect these by checking
/// for a `]` character immediately before the operator.
///
/// Quick check (15 repos): resolved 138 FP and 24 FN with 0 regressions.
///
/// ## Corpus fix (2026-04-06)
///
/// The alignment check functions (`check_alignment_standalone`,
/// `check_rhs_alignment_standalone`, `has_subsequent_assignment_neighbor`)
/// were incorrectly using lines starting with `^` (annotation markers in test
/// fixtures) as alignment references. This caused extra-space offenses to be
/// incorrectly suppressed when the `=` or `=>` on the source line aligned
/// with the `=` in an annotation line (e.g., `^ Layout/SpaceAroundOperators:
/// Operator = should be surrounded by a single space.`).
///
/// Fix: added `Some(fs) if line_bytes[fs] == b'^' => {}` to skip annotation
/// lines in all three alignment functions, matching the existing `#` comment
/// skip behavior.
///
/// Sample check (15 repos): resolved 29 FP and 29 FN with 0 regressions.
///
/// ## Corpus fix (2026-04-08)
///
/// Two fixes applied:
///
/// 1. **Endless method `=` exclusion** (FP fix): The `=` in endless method
///    definitions (`def foo = expr`) was being flagged by the text scanner as
///    an assignment with extra leading space. RuboCop excludes these via
///    `remove_equals_in_def` in `PrecedingFollowingAlignment`. Fixed by
///    collecting `DefNode.equal_loc()` offsets in `ExclusionCollector` and
///    skipping them in the text scanner. This resolves ~99 FP from the
///    syntax_tree repo and others.
///
/// 2. **Assignment neighbor search skips non-assignment lines** (FN fix):
///    `has_subsequent_assignment_neighbor` was stopping at the first same-indent
///    non-assignment line (e.g., `foo(bar)` between two assignments). RuboCop's
///    `relevant_assignment_lines` continues past non-assignment lines to find
///    the next assignment. Replaced with `should_flag_assignment_extra_leading_space`
///    which also checks the preceding assignment for alignment and handles the
///    blank-line termination logic from RuboCop's `relevant_line_indent_at_level`.
///
/// Sample check (15 repos): resolved 16 FP and 63 FN with 0 regressions.
///
/// ## Corpus fix (2026-04-08, setter/index writes)
///
/// Prism parses `obj.attr = value` and `hash[:key] = value` as `CallNode`
/// attribute writes with the standalone `=` stored in `equal_loc()`. The text
/// scanner was still handling those `=` tokens generically, which missed
/// RuboCop's context-sensitive alignment behavior for setter/index writes.
///
/// Fix: visit `CallNode` attribute writes directly, report `equal_loc()` via the
/// AST pass, and use the first argument's start offset as the trailing alignment
/// anchor. This matches RuboCop's `on_setter_method` behavior closely enough to
/// accept spaced-key cases like `content[ :query  ] =  ...` without weakening
/// existing plain-assignment or wide-padding checks.
///
/// ## Corpus fix (2026-04-10)
///
/// Two false-positive patterns needed narrower RuboCop matching:
/// - mixed `tab + spaces` before `=` (for example `Base32\t     = ...`) are
///   accepted because RuboCop only treats leading whitespace as "excessive"
///   when the entire whitespace run starts with two spaces.
/// - plain-assignment alignment search must only treat `=` and operator-write
///   tokens as assignment neighbors. Comparisons like `>=` and `===` do not
///   participate in RuboCop's `aligned_with_preceding/subsequent_equals_operator`
///   checks, and counting them caused standalone `=` assignments to be flagged.
///
/// ## Corpus fix (2026-04-11)
///
/// RuboCop treats `||=` and `&&=` like plain assignments for extra leading
/// space: standalone `@config  ||= foo` and `self.state  ||= nil` are accepted,
/// but extra trailing space (`x ||=  0`) is still an offense. The AST path was
/// incorrectly checking `||=`/`&&=` like generic operator assignments, which
/// flagged leading-only spacing that RuboCop allows.
///
/// ## Corpus fix (2026-04-12)
///
/// Plain-assignment leading space needs RuboCop's dedicated neighbor search,
/// not the generic adjacent-operator alignment shortcut. Using the generic path
/// incorrectly suppressed offenses across blank-line-separated groups
/// (`tag    = ...`, `ems1      = ...`) and against same-line chained writes
/// (`Reline.input  = @input  = ...`, `SetUIDBit = ReadBit  = 4`). RuboCop
/// only considers the first assignment token on each neighbor line here.
///
/// ## Corpus fix (2026-04-17)
///
/// RuboCop treats newline edges specially:
/// - operators at the beginning of a line are accepted (`"a"\n-\n"b"`,
///   zero-indent continuation `+ 1`, and syntax-tree fixtures where `%` starts
///   the line),
/// - extra spaces after an operator are accepted when the operator is the last
///   token on the line (`foo +            \n  bar`).
///
/// nitrocop's AST path and text scanner were still enforcing normal spacing at
/// those boundaries, which produced false positives in repos like syntax_tree,
/// add_to_calendar, and lamernews. Match RuboCop by treating line-start
/// operators as already having valid leading space and by ignoring trailing
/// padding that is followed only by a newline.
///
/// ## Corpus fix (2026-04-17, keyword logical operators)
///
/// Prism represents keyword logical operators (`and`, `or`) with the same
/// `AndNode`/`OrNode` types as symbolic `&&`/`||`, but the visitor was
/// explicitly skipping the keyword forms. RuboCop aliases `on_and`/`on_or`
/// to `on_binary`, so those keyword operators should use the normal binary
/// spacing rules, including extra-space checks in modifier-return and
/// multiline-condition contexts.
///
/// ## Corpus fix (2026-04-18)
///
/// Plain `=` assignments with a continued multiline RHS need trailing-space
/// alignment to start from the actual RHS node, not the first non-space byte
/// on the assignment line. RuboCop therefore accepts webmock-style code like
/// `expected =  \` followed by aligned string continuation lines, while the
/// old text-scanner path anchored on the backslash and falsely flagged it.
/// Fix: handle plain assignment write nodes in the AST pass and use
/// `value().location().start_offset()` as the trailing alignment anchor.
///
/// ## Corpus fix (2026-04-18, adjacent-line cross alignment)
///
/// RuboCop's generic `aligned_with_operator?` path is narrower than nitrocop's
/// old raw-byte check:
/// - only `<<` and operators ending with `=` may use cross-operator alignment,
/// - the adjacent line only contributes its *first* eligible
///   assignment/comparison token.
///
/// The broader scanner was suppressing real offenses in modifier conditions and
/// comparison lines, for example `@become              != true` aligned against
/// a later `==` on the next line, or `max_retries  > 0` aligned against a
/// preceding assignment `=`. Match RuboCop by preserving identical-operator
/// alignment but restricting cross-operator alignment to the first eligible
/// neighbor token.
///
/// ## Corpus fix (2026-04-19)
///
/// Two narrow false-positive fixes were needed to match RuboCop:
/// - `==`/`!=` written with explicit dot-call syntax still stay out of scope
///   for this cop even when spaces appear between `.` and the operator
///   (`pixels(...).  == Array.new(300)`).
/// - plain regexp literals on the left side of `=~` are also accepted without
///   spacing (`assert(/Fred/=~xml)`), but RuboCop still flags `/.../!~expr`
///   and interpolated regexp receivers like `/#{foo}/=~x`.
///
/// ## Corpus fix (2026-04-19, rational literals and `$=`)
///
/// Two more false-positive buckets required narrower matching:
/// - RuboCop only exempts structural `(int) / (rational)` sends such as
///   `5 / 3r`; generic expressions like `a * b / 42r` still follow the
///   configured rational-literal style. The previous byte-based check treated
///   every `/ ...r` as a rational literal and diverged on integer/rational
///   literal forms.
/// - The predefined global `$=` is a variable name, not an operator token.
///   The text scanner was interpreting the `=` inside `$=` as a standalone
///   operator and reporting missing-space offenses on reads and writes.
///
/// ## Corpus fix (2026-04-19, repeated `=>` call arguments)
///
/// RuboCop accepts extra trailing space after `=>` when the pair lines up with
/// an adjacent pair in one of two ways:
/// - normal hash-pair alignment at the pair start (space/non-space boundary),
/// - or an exact same-column pair-source match, which is what repeated call
///   arguments like `have_solution(x =>  true)` and repeated
///   `where(:to_org_id =>  Org...)` pairs rely on.
///
/// The previous implementation only checked the boundary case for `AssocNode`,
/// so it still flagged repeated call-argument pairs that RuboCop accepts. Keep
/// the boundary check for multiline hash alignment, and also allow an exact
/// full-pair match at the same column for `AssocNode` trailing-space checks.
///
/// A tempting isolated FN fixture, `tree1      = BTree[...]`, was also checked
/// during this investigation. RuboCop accepts that line in isolation, so it is
/// not modeled as a standalone offense here.
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
    /// Byte offsets of `=` in endless method definitions (e.g., `def foo = expr`).
    /// RuboCop excludes these from `assignment_tokens` via `remove_equals_in_def`.
    endless_def_equal_offsets: HashSet<usize>,
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
        // Collect `=` offsets from endless method definitions (def foo = expr).
        // RuboCop excludes these from assignment_tokens via remove_equals_in_def.
        if let Some(equal_loc) = node.equal_loc() {
            self.endless_def_equal_offsets
                .insert(equal_loc.start_offset());
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
            endless_def_equal_offsets: HashSet::new(),
        };
        collector.visit(&parse_result.node());
        let default_param_offsets = collector.default_param_offsets;
        let plain_assignment_offsets = collector.plain_assignment_offsets;
        let endless_def_equal_offsets = collector.endless_def_equal_offsets;
        let def_name_ranges = collector.def_method_name_ranges;

        let exponent_no_space = enforced_style_exponent == "no_space";
        let rational_no_space = enforced_style_rational == "no_space";

        // AST-based check for binary operators, assignments, and other operator nodes.
        let mut op_checker = OperatorChecker {
            cop: self,
            source,
            code_map,
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

                    // Skip method calls via `.` or `&.`: e.g., `x&.!= y`,
                    // `x.== y`, or `x.  == y`.
                    if uses_dot_operator_call_syntax(bytes, i) {
                        i += 2;
                        continue;
                    }

                    let op_str = std::str::from_utf8(two).unwrap_or("??");
                    let space_before = is_operator_at_line_start(bytes, i)
                        || (i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t'));
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
                        let multi_before = has_excessive_leading_space(bytes, i);
                        let multi_after = has_excessive_trailing_space(bytes, i + 2);
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
                // Skip the predefined global variable `$=` — the `=` is part of
                // the variable name, not an operator token.
                if i > 0 && bytes[i - 1] == b'$' {
                    i += 1;
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

                // Skip `=` in endless method definitions (def foo = expr).
                // RuboCop excludes these from assignment_tokens via remove_equals_in_def.
                if endless_def_equal_offsets.contains(&i) {
                    i += 1;
                    continue;
                }

                // Skip `=` that is part of an operator method name: `def []=`, `def ===`
                if in_def_name(i) {
                    i += 1;
                    continue;
                }

                // Skip `=` that is part of an explicit `.[]=` method call with dot
                // syntax (e.g., `@flows.[]=(*args)`).  RuboCop treats `[]=` as an
                // irregular method and doesn't check it as an operator.
                // Do NOT skip regular index assignments like `hash[:key]= value`
                // — RuboCop checks those via on_setter_method.
                if i >= 3 && bytes[i - 1] == b']' && bytes[i - 2] == b'[' && bytes[i - 3] == b'.' {
                    i += 1;
                    continue;
                }

                let space_before = is_operator_at_line_start(bytes, i)
                    || (i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t'));
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
                    let multi_before = has_excessive_leading_space(bytes, i);
                    let multi_after = has_excessive_trailing_space(bytes, i + 1);
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
    let mut multi_before = multi_before && has_excessive_leading_space(bytes, op_start);
    let mut multi_after = multi_after && has_excessive_trailing_space(bytes, op_end);

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

    if multi_before
        && !is_plain_assignment
        && is_aligned_standalone(source, op_start, op_bytes, code_map)
    {
        multi_before = false;
    }

    // RuboCop-compatible: for plain assignments (=), only flag extra leading space
    // when there is a subsequent assignment at the same indentation that is NOT
    // aligned with the current operator. Mirrors RuboCop's excess_leading_space?
    // logic for :assignment type.
    if multi_before && is_plain_assignment {
        multi_before =
            should_flag_assignment_extra_leading_space(source, op_start, op_bytes, code_map);
    }

    if multi_after {
        let mut p = op_end;
        while p < bytes.len() && bytes[p] == b' ' {
            p += 1;
        }
        if p >= bytes.len() || bytes[p] == b'\n' || bytes[p] == b'\r' || bytes[p] == b'#' {
            multi_after = false;
        } else {
            // Check RHS alignment for trailing space.  RuboCop's
            // `excess_trailing_space?` uses `aligned_with_something?` on the
            // right operand for ALL operator types.  For index writes like
            // `x[:key] = value`, RuboCop still flags the trailing space even
            // when the RHS values align, because the operator itself is not
            // at the expected column.  We approximate this by skipping the
            // alignment check for `=` where the non-space character before
            // the operator is `]` (index write pattern).
            let is_index_write_eq = op_bytes == b"=" && !is_plain_assignment && {
                let mut j = op_start;
                while j > 0 && (bytes[j - 1] == b' ' || bytes[j - 1] == b'\t') {
                    j -= 1;
                }
                j > 0 && bytes[j - 1] == b']'
            };

            if !is_index_write_eq {
                if let Some(rhs_start) = util::first_non_space_on_line(bytes, op_end) {
                    if is_aligned_rhs_standalone(source, rhs_start, true, None) {
                        multi_after = false;
                    }
                }
            }
        }
    }

    if !multi_before && !multi_after {
        return;
    }

    let ws_start = if multi_before {
        whitespace_run_start(bytes, op_start)
    } else {
        op_start
    };
    let ws_end = if multi_after {
        whitespace_run_end(bytes, op_end)
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

fn whitespace_run_start(bytes: &[u8], offset: usize) -> usize {
    let mut start = offset;
    while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
        start -= 1;
    }
    start
}

fn whitespace_run_end(bytes: &[u8], offset: usize) -> usize {
    let mut end = offset;
    while end < bytes.len() && matches!(bytes[end], b' ' | b'\t') {
        end += 1;
    }
    end
}

fn has_excessive_leading_space(bytes: &[u8], op_start: usize) -> bool {
    let ws_start = whitespace_run_start(bytes, op_start);
    op_start.saturating_sub(ws_start) >= 2 && bytes[ws_start] == b' ' && bytes[ws_start + 1] == b' '
}

fn uses_dot_operator_call_syntax(bytes: &[u8], op_start: usize) -> bool {
    let mut pos = op_start;
    while pos > 0 && matches!(bytes[pos - 1], b' ' | b'\t') {
        pos -= 1;
    }

    pos > 0 && bytes[pos - 1] == b'.'
}

fn has_excessive_trailing_space(bytes: &[u8], op_end: usize) -> bool {
    let ws_end = whitespace_run_end(bytes, op_end);
    ws_end.saturating_sub(op_end) >= 2 && bytes[ws_end - 2] == b' ' && bytes[ws_end - 1] == b' '
}

fn line_start_offset(bytes: &[u8], offset: usize) -> usize {
    let mut start = offset;
    while start > 0 && bytes[start - 1] != b'\n' {
        start -= 1;
    }
    start
}

fn is_operator_at_line_start(bytes: &[u8], offset: usize) -> bool {
    let line_start = line_start_offset(bytes, offset);
    bytes[line_start..offset]
        .iter()
        .all(|&b| b == b' ' || b == b'\t')
}

fn followed_only_by_space_then_newline(bytes: &[u8], offset: usize) -> bool {
    let mut pos = offset;
    while pos < bytes.len() && bytes[pos] == b' ' {
        pos += 1;
    }
    pos >= bytes.len() || bytes[pos] == b'\n' || bytes[pos] == b'\r'
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
fn is_aligned_standalone(
    source: &SourceFile,
    start: usize,
    op_bytes: &[u8],
    code_map: &CodeMap,
) -> bool {
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
    if check_alignment_standalone(
        &lines,
        line_idx,
        char_col,
        char_end_col,
        op_bytes,
        None,
        code_map,
    ) {
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
        code_map,
    )
}

fn check_alignment_standalone(
    lines: &[&[u8]],
    line_idx: usize,
    char_col: usize,
    char_end_col: usize,
    op_bytes: &[u8],
    indent_filter: Option<usize>,
    code_map: &CodeMap,
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
                Some(fs) if line_bytes[fs] == b'^' => {} // Annotation line — skip
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
                    // Check 3: cross-operator alignment only applies to `<<` and
                    // operators ending with `=` and only considers the first eligible
                    // assignment/comparison token on the adjacent line.
                    if line_has_cross_aligned_operator_at_char_col(
                        line_bytes,
                        line_abs_start(lines, check_idx),
                        char_end_col,
                        op_bytes,
                        code_map,
                    ) {
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum AlignmentTokenKind {
    EqualLike,
    LShift,
}

fn current_operator_allows_cross_alignment(op_bytes: &[u8]) -> bool {
    op_bytes == b"<<" || op_bytes.last() == Some(&b'=')
}

fn line_has_cross_aligned_operator_at_char_col(
    line: &[u8],
    line_abs_start: usize,
    target_char_end_col: usize,
    current_op: &[u8],
    code_map: &CodeMap,
) -> bool {
    if !current_operator_allows_cross_alignment(current_op) {
        return false;
    }

    let Some((kind, end_char_col)) = first_alignment_token_on_line(line, line_abs_start, code_map)
    else {
        return false;
    };

    if end_char_col != target_char_end_col {
        return false;
    }

    if current_op == b"<<" {
        kind == AlignmentTokenKind::EqualLike
    } else {
        matches!(
            kind,
            AlignmentTokenKind::EqualLike | AlignmentTokenKind::LShift
        )
    }
}

fn line_abs_start(lines: &[&[u8]], line_idx: usize) -> usize {
    lines.iter().take(line_idx).map(|line| line.len() + 1).sum()
}

fn first_alignment_token_on_line(
    line: &[u8],
    line_abs_start: usize,
    code_map: &CodeMap,
) -> Option<(AlignmentTokenKind, usize)> {
    let equal_like = first_equal_like_alignment_token(line, line_abs_start, code_map)
        .map(|(start, end_char_col)| (AlignmentTokenKind::EqualLike, start, end_char_col));
    let lshift = first_lshift_alignment_token(line, line_abs_start, code_map)
        .map(|(start, end_char_col)| (AlignmentTokenKind::LShift, start, end_char_col));

    match (equal_like, lshift) {
        (Some(eq), Some(ls)) => {
            if eq.1 < ls.1 {
                Some((eq.0, eq.2))
            } else {
                Some((ls.0, ls.2))
            }
        }
        (Some(eq), None) => Some((eq.0, eq.2)),
        (None, Some(ls)) => Some((ls.0, ls.2)),
        (None, None) => None,
    }
}

fn first_equal_like_alignment_token(
    line: &[u8],
    line_abs_start: usize,
    code_map: &CodeMap,
) -> Option<(usize, usize)> {
    let three_char_ops: [&[u8]; 6] = [b"===", b"<<=", b">>=", b"||=", b"&&=", b"**="];
    let two_char_ops: [&[u8]; 12] = [
        b"==", b"!=", b"<=", b">=", b"+=", b"-=", b"*=", b"/=", b"%=", b"^=", b"|=", b"&=",
    ];

    for i in 0..line.len() {
        let abs_offset = line_abs_start + i;
        if !code_map.is_code(abs_offset) {
            continue;
        }

        if i + 3 <= line.len() && three_char_ops.contains(&&line[i..i + 3]) {
            return Some((i, bytes_to_char_col(line, i + 3)));
        }

        if i + 2 <= line.len() {
            let two = &line[i..i + 2];
            if two_char_ops.contains(&two) {
                return Some((i, bytes_to_char_col(line, i + 2)));
            }
        }

        if line[i] == b'=' && is_plain_equal_alignment_token(line, i) {
            return Some((i, bytes_to_char_col(line, i + 1)));
        }
    }

    None
}

fn first_lshift_alignment_token(
    line: &[u8],
    line_abs_start: usize,
    code_map: &CodeMap,
) -> Option<(usize, usize)> {
    for i in 0..line.len().saturating_sub(1) {
        let abs_offset = line_abs_start + i;
        if !code_map.is_code(abs_offset) {
            continue;
        }

        if &line[i..i + 2] == b"<<" && (i + 2 >= line.len() || line[i + 2] != b'=') {
            return Some((i, bytes_to_char_col(line, i + 2)));
        }
    }

    None
}

fn is_plain_equal_alignment_token(line: &[u8], i: usize) -> bool {
    if i + 1 < line.len() && matches!(line[i + 1], b'>' | b'=' | b'~') {
        return false;
    }

    if i == 0 {
        return false;
    }

    !matches!(
        line[i - 1],
        b'!' | b'<' | b'>' | b'=' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'~'
    )
}

/// RuboCop-compatible check for plain assignment extra leading space.
///
/// Returns `true` if the offense should be flagged, `false` if suppressed.
///
/// Mirrors RuboCop's `excess_leading_space?` for `:assignment` type:
/// 1. If preceding assignment at same indent is aligned → suppress
/// 2. If no subsequent assignment at same indent → suppress (`:none`)
/// 3. If subsequent assignment is aligned → suppress (`:yes`)
/// 4. If subsequent assignment is NOT aligned → flag (`:no`)
fn should_flag_assignment_extra_leading_space(
    source: &SourceFile,
    op_start: usize,
    op_bytes: &[u8],
    code_map: &CodeMap,
) -> bool {
    let bytes = source.as_bytes();
    let lines: Vec<&[u8]> = source.lines().collect();
    let (line, _) = source.offset_to_line_col(op_start);
    let line_idx = line - 1;

    // Compute the character end column of the operator (for cross-operator alignment)
    let mut ls = op_start;
    while ls > 0 && bytes[ls - 1] != b'\n' {
        ls -= 1;
    }
    let byte_col = op_start - ls;
    let char_end_col = bytes_to_char_col(lines[line_idx], byte_col) + op_bytes.len();

    let my_indent = lines[line_idx]
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(0);

    let line_starts = compute_line_starts(bytes);

    // Check preceding: if the nearest preceding assignment is aligned, suppress
    if find_assignment_aligned_in_direction(
        &lines,
        &line_starts,
        line_idx,
        my_indent,
        char_end_col,
        code_map,
        true,
    ) == Some(true)
    {
        return false;
    }

    // Check subsequent: None → suppress, Aligned → suppress, NotAligned → flag
    match find_assignment_aligned_in_direction(
        &lines,
        &line_starts,
        line_idx,
        my_indent,
        char_end_col,
        code_map,
        false,
    ) {
        None => false,       // no subsequent assignment
        Some(true) => false, // subsequent is aligned
        Some(false) => true, // subsequent is NOT aligned → flag
    }
}

/// Search for the nearest assignment line at the same indentation in the given
/// direction (up or down), skipping non-assignment same-indent lines.
///
/// Returns `None` if no assignment found, `Some(true)` if found and aligned
/// (operator end column matches), `Some(false)` if found and not aligned.
///
/// Mirrors RuboCop's `relevant_assignment_lines` + `aligned_equals_operator?`.
fn find_assignment_aligned_in_direction(
    lines: &[&[u8]],
    line_starts: &[usize],
    line_idx: usize,
    my_indent: usize,
    char_end_col: usize,
    code_map: &CodeMap,
    search_up: bool,
) -> Option<bool> {
    let mut check_idx = if search_up {
        if line_idx == 0 {
            return None;
        }
        line_idx - 1
    } else {
        line_idx + 1
    };

    // Track whether the last non-blank line was at the target indent,
    // used for blank-line termination (mirrors RuboCop's relevant_line_indent_at_level).
    let mut relevant_indent_at_level = true;

    loop {
        if check_idx >= lines.len() {
            break;
        }
        let line_bytes = lines[check_idx];
        let first_non_ws = line_bytes.iter().position(|&b| b != b' ' && b != b'\t');
        match first_non_ws {
            None => {
                // Blank line: terminates search if last non-blank was at same indent
                if relevant_indent_at_level {
                    break;
                }
            }
            Some(fs) if line_bytes[fs] == b'#' => {} // Comment line — skip
            Some(fs) if line_bytes[fs] == b'^' => {} // Annotation line — skip
            Some(indent) => {
                if indent < my_indent {
                    break; // Dedented — stop
                }
                if indent == my_indent {
                    relevant_indent_at_level = true;
                    let abs_start = line_starts[check_idx];
                    if let Some(first_end_col) =
                        first_assignment_alignment_end_char_col(line_bytes, abs_start, code_map)
                    {
                        return Some(first_end_col == char_end_col);
                    }
                    // Same-indent non-assignment line — continue searching
                } else {
                    // More indented — continuation, don't terminate
                    relevant_indent_at_level = false;
                }
            }
        }
        if search_up {
            if check_idx == 0 {
                break;
            }
            check_idx -= 1;
        } else {
            check_idx += 1;
        }
    }
    None
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

/// Return the character end column of the first assignment token on a line.
///
/// This mirrors RuboCop's `assignment_tokens.uniq(&:line)`, which only lets
/// the first `=`/operator-assignment token on each line participate in plain
/// assignment alignment checks.
fn first_assignment_operator_end_char_col(
    line: &[u8],
    line_abs_start: usize,
    code_map: &CodeMap,
) -> Option<usize> {
    for i in 0..line.len() {
        if line[i] != b'=' {
            continue;
        }

        let abs_offset = line_abs_start + i;
        if !code_map.is_code(abs_offset) {
            continue;
        }

        if i + 1 < line.len() && matches!(line[i + 1], b'>' | b'=' | b'~') {
            continue;
        }

        if i == 0 {
            continue;
        }

        let prev = line[i - 1];
        let is_assignment_token = matches!(prev, b' ' | b'\t')
            || prev.is_ascii_alphanumeric()
            || matches!(prev, b'_' | b')' | b']')
            || matches!(prev, b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'|' | b'&')
            || (matches!(prev, b'<' | b'>') && i >= 2 && line[i - 2] == prev);
        if !is_assignment_token {
            continue;
        }

        return Some(bytes_to_char_col(line, i + 1));
    }
    None
}

/// Return the character end column of the first alignment token on a line that
/// also contains a plain/compound assignment `=` token somewhere on that line.
///
/// RuboCop's plain-assignment alignment first filters candidate neighbor lines
/// through `assignment_tokens` (lines that contain an equal-sign assignment),
/// then checks alignment against the first eligible alignment token on that
/// line via `aligned_equals_operator?`, which can be `<<` as well as `=`, `==`,
/// `+=`, etc.
fn first_assignment_alignment_end_char_col(
    line: &[u8],
    line_abs_start: usize,
    code_map: &CodeMap,
) -> Option<usize> {
    first_assignment_operator_end_char_col(line, line_abs_start, code_map)?;
    first_alignment_token_on_line(line, line_abs_start, code_map)
        .map(|(_, end_char_col)| end_char_col)
}

fn is_aligned_rhs_standalone(
    source: &SourceFile,
    start: usize,
    token_match: bool,
    exact_token: Option<&[u8]>,
) -> bool {
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
    // Only pass current_line for Check 2 (exact token match) when token_match
    // is enabled.  For trailing_anchor paths (e.g. `=>` pair key position),
    // the anchor is the hash key, not the value — short key names like `x`
    // would spuriously match on adjacent lines.
    let current_line = if token_match {
        Some(lines[line_idx])
    } else {
        None
    };

    if check_rhs_alignment_standalone(&lines, line_idx, char_col, None, current_line, exact_token) {
        return true;
    }

    let my_indent = lines[line_idx]
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(0);
    check_rhs_alignment_standalone(
        &lines,
        line_idx,
        char_col,
        Some(my_indent),
        current_line,
        exact_token,
    )
}

fn check_rhs_alignment_standalone(
    lines: &[&[u8]],
    line_idx: usize,
    char_col: usize,
    indent_filter: Option<usize>,
    current_line: Option<&[u8]>,
    exact_token: Option<&[u8]>,
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
                Some(fs) if line_bytes[fs] == b'^' => {} // Annotation line — skip
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

                    if line_has_aligned_rhs_at_char_col(
                        line_bytes,
                        char_col,
                        current_line,
                        exact_token,
                    ) {
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

/// Checks RHS alignment on an adjacent line using RuboCop's `aligned_words?` logic:
/// 1. Space/tab + non-space boundary at `target_char_col - 1` to `target_char_col`
/// 2. Exact token match: the same text starting at `target_char_col` on both lines
///    (only when `current_line` is `Some`)
fn line_has_aligned_rhs_at_char_col(
    line: &[u8],
    target_char_col: usize,
    current_line: Option<&[u8]>,
    exact_token: Option<&[u8]>,
) -> bool {
    let Some(byte_col) = char_col_to_bytes(line, target_char_col) else {
        return false;
    };

    // Check 1: space/non-space boundary (RuboCop: /\s\S/)
    if byte_col > 0
        && byte_col < line.len()
        && (line[byte_col - 1] == b' ' || line[byte_col - 1] == b'\t')
        && line[byte_col] != b' '
        && line[byte_col] != b'\t'
    {
        return true;
    }

    // Check 2: exact token match at the same column (RuboCop: token == line[left_edge, len])
    // Only applied when current_line is provided (disabled for trailing_anchor
    // paths where the anchor is a hash key, not the RHS value).
    if let Some(token) = exact_token {
        if byte_col + token.len() <= line.len() {
            return line[byte_col..byte_col + token.len()] == *token;
        }
        return false;
    }

    let Some(cur_line) = current_line else {
        return false;
    };
    let Some(current_byte_col) = char_col_to_bytes(cur_line, target_char_col) else {
        return false;
    };
    if current_byte_col >= cur_line.len() || byte_col >= line.len() {
        return false;
    }
    // Extract the token from the current line (until whitespace or end of line)
    let token_end = cur_line[current_byte_col..]
        .iter()
        .position(|&b| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r')
        .map_or(cur_line.len(), |p| current_byte_col + p);
    let token_len = token_end - current_byte_col;
    if token_len > 0 && byte_col + token_len <= line.len() {
        return line[byte_col..byte_col + token_len]
            == cur_line[current_byte_col..current_byte_col + token_len];
    }
    false
}

const BINARY_OPERATORS: &[&[u8]] = &[
    b"+", b"-", b"*", b"/", b"%", b"**", b"&", b"|", b"^", b"<<", b">>", b"<", b">", b"<=", b">=",
    b"<=>",
];

/// Additional operators detected via CallNode (match operators, ===)
const MATCH_OPERATORS: &[&[u8]] = &[b"=~", b"!~", b"==="];

#[derive(Clone)]
struct TrailingAnchor {
    offset: usize,
    token_match: bool,
    exact_token: Option<Vec<u8>>,
}

impl TrailingAnchor {
    fn is_aligned(&self, source: &SourceFile) -> bool {
        is_aligned_rhs_standalone(
            source,
            self.offset,
            self.token_match,
            self.exact_token.as_deref(),
        )
    }
}

struct OperatorChecker<'a> {
    cop: &'a SpaceAroundOperators,
    source: &'a SourceFile,
    code_map: &'a CodeMap,
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
        is_aligned_standalone(self.source, start, op_bytes, self.code_map)
    }

    /// Check operator spacing for a "should have space" operator.
    /// Reports missing space or extra space around the operator.
    fn check_operator_spacing(&mut self, op_loc: &ruby_prism::Location<'_>) {
        self.check_operator_spacing_with_trailing_anchor(op_loc, None, false);
    }

    /// RuboCop treats `||=` and `&&=` like plain assignments for extra leading
    /// space, but still enforces trailing-space rules.
    fn check_assignment_like_operator_spacing(&mut self, op_loc: &ruby_prism::Location<'_>) {
        self.check_operator_spacing_with_trailing_anchor(op_loc, None, true);
    }

    fn check_plain_assignment_spacing(
        &mut self,
        op_loc: &ruby_prism::Location<'_>,
        value: &ruby_prism::Node<'_>,
    ) {
        self.reported_offsets.insert(op_loc.start_offset());
        self.check_operator_spacing_with_trailing_anchor(
            op_loc,
            Some(TrailingAnchor {
                offset: value.location().start_offset(),
                token_match: true,
                exact_token: None,
            }),
            true,
        );
    }

    fn check_operator_spacing_with_trailing_anchor(
        &mut self,
        op_loc: &ruby_prism::Location<'_>,
        trailing_anchor: Option<TrailingAnchor>,
        assignment_like_leading: bool,
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

        let has_space_before = is_operator_at_line_start(bytes, start)
            || (start > 0 && (bytes[start - 1] == b' ' || bytes[start - 1] == b'\t'));
        let has_space_after = end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'\t');
        let newline_after = end >= bytes.len() || bytes[end] == b'\n' || bytes[end] == b'\r';

        // Accept tabs as spacing (RuboCop: "accepts operator surrounded by tabs")
        if has_space_before && (has_space_after || newline_after) {
            // Check for multiple spaces (extra whitespace before or after operator)
            let multi_space_before = has_excessive_leading_space(bytes, start);
            let multi_space_after = has_excessive_trailing_space(bytes, end);

            if multi_space_before || multi_space_after {
                self.check_extra_space(
                    start,
                    end,
                    op_str,
                    op_loc.as_slice(),
                    trailing_anchor,
                    assignment_like_leading,
                );
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
        trailing_anchor: Option<TrailingAnchor>,
        assignment_like_leading: bool,
    ) {
        let bytes = self.source.as_bytes();
        let mut multi_space_before = has_excessive_leading_space(bytes, start);
        let mut multi_space_after = has_excessive_trailing_space(bytes, end);

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
            && !assignment_like_leading
            && self.is_aligned_with_adjacent(start, op_bytes)
        {
            multi_space_before = false;
        }

        if multi_space_before && assignment_like_leading {
            multi_space_before = should_flag_assignment_extra_leading_space(
                self.source,
                start,
                op_bytes,
                self.code_map,
            );
        }

        if multi_space_after {
            let mut p = end;
            while p < bytes.len() && bytes[p] == b' ' {
                p += 1;
            }
            if followed_only_by_space_then_newline(bytes, end)
                || (p < bytes.len() && bytes[p] == b'#')
            {
                multi_space_after = false;
            } else if self.allow_for_alignment {
                if let Some(anchor) = trailing_anchor {
                    if anchor.is_aligned(self.source) {
                        multi_space_after = false;
                    }
                } else if let Some(rhs_start) = util::first_non_space_on_line(bytes, end) {
                    if is_aligned_rhs_standalone(self.source, rhs_start, true, None) {
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
            whitespace_run_start(bytes, start)
        } else {
            start
        };
        // Find the extent of extra spaces after the operator
        let ws_end_after = if multi_space_after {
            whitespace_run_end(bytes, end)
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

    fn slash_has_rational_argument(&self, node: &ruby_prism::CallNode<'_>) -> bool {
        let Some(arguments) = node.arguments() else {
            return false;
        };
        let mut args = arguments.arguments().iter();
        let Some(first) = args.next() else {
            return false;
        };
        first.as_rational_node().is_some() && args.next().is_none()
    }

    /// RuboCop's `RationalLiteral` mixin only exempts structural `(int) / (rational)`
    /// sends from normal operator spacing checks.
    fn is_rational_literal_call(&self, node: &ruby_prism::CallNode<'_>) -> bool {
        node.name().as_slice() == b"/"
            && node
                .receiver()
                .is_some_and(|receiver| receiver.as_integer_node().is_some())
            && self.slash_has_rational_argument(node)
    }
}

impl<'pr> Visit<'pr> for OperatorChecker<'_> {
    // === Binary operators via CallNode (including match operators and ===) ===
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if node.is_attribute_write() {
            if let Some(equal_loc) = node.equal_loc() {
                self.reported_offsets.insert(equal_loc.start_offset());

                let trailing_anchor = node
                    .arguments()
                    .and_then(|args| args.arguments().iter().next())
                    .map(|arg| arg.location().start_offset());

                self.check_operator_spacing_with_trailing_anchor(
                    &equal_loc,
                    trailing_anchor.map(|offset| TrailingAnchor {
                        offset,
                        token_match: true,
                        exact_token: None,
                    }),
                    false,
                );
            }

            ruby_prism::visit_call_node(self, node);
            return;
        }

        let name = node.name().as_slice();

        if self.is_rational_literal_call(node) {
            ruby_prism::visit_call_node(self, node);
            return;
        }

        if name == b"=~"
            && node
                .receiver()
                .is_some_and(|receiver| receiver.as_regular_expression_node().is_some())
        {
            ruby_prism::visit_call_node(self, node);
            return;
        }

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
                        && self.slash_has_rational_argument(node));
                if should_have_no_space {
                    self.check_no_space_operator(&msg_loc);
                } else {
                    self.check_operator_spacing(&msg_loc);
                }
            }
        }

        ruby_prism::visit_call_node(self, node);
    }

    // === Plain assignments (`=`) ===
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        let value = node.value();
        self.check_plain_assignment_spacing(&node.operator_loc(), &value);
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_instance_variable_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableWriteNode<'pr>,
    ) {
        let value = node.value();
        self.check_plain_assignment_spacing(&node.operator_loc(), &value);
        ruby_prism::visit_instance_variable_write_node(self, node);
    }

    fn visit_class_variable_write_node(&mut self, node: &ruby_prism::ClassVariableWriteNode<'pr>) {
        let value = node.value();
        self.check_plain_assignment_spacing(&node.operator_loc(), &value);
        ruby_prism::visit_class_variable_write_node(self, node);
    }

    fn visit_global_variable_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableWriteNode<'pr>,
    ) {
        let value = node.value();
        self.check_plain_assignment_spacing(&node.operator_loc(), &value);
        ruby_prism::visit_global_variable_write_node(self, node);
    }

    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        let value = node.value();
        self.check_plain_assignment_spacing(&node.operator_loc(), &value);
        ruby_prism::visit_constant_write_node(self, node);
    }

    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'pr>) {
        let value = node.value();
        self.check_plain_assignment_spacing(&node.operator_loc(), &value);
        ruby_prism::visit_constant_path_write_node(self, node);
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        let value = node.value();
        self.check_plain_assignment_spacing(&node.operator_loc(), &value);
        ruby_prism::visit_multi_write_node(self, node);
    }

    // === Logical operators (&&, ||) ===
    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
        ruby_prism::visit_and_node(self, node);
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        self.check_operator_spacing(&node.operator_loc());
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
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_local_variable_or_write_node(self, node);
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_local_variable_and_write_node(self, node);
    }

    fn visit_instance_variable_or_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOrWriteNode<'pr>,
    ) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_instance_variable_or_write_node(self, node);
    }

    fn visit_instance_variable_and_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableAndWriteNode<'pr>,
    ) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_instance_variable_and_write_node(self, node);
    }

    fn visit_class_variable_or_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOrWriteNode<'pr>,
    ) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_class_variable_or_write_node(self, node);
    }

    fn visit_class_variable_and_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableAndWriteNode<'pr>,
    ) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_class_variable_and_write_node(self, node);
    }

    fn visit_global_variable_or_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOrWriteNode<'pr>,
    ) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_global_variable_or_write_node(self, node);
    }

    fn visit_global_variable_and_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableAndWriteNode<'pr>,
    ) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_global_variable_and_write_node(self, node);
    }

    fn visit_constant_or_write_node(&mut self, node: &ruby_prism::ConstantOrWriteNode<'pr>) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_constant_or_write_node(self, node);
    }

    fn visit_constant_and_write_node(&mut self, node: &ruby_prism::ConstantAndWriteNode<'pr>) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_constant_and_write_node(self, node);
    }

    fn visit_constant_path_or_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOrWriteNode<'pr>,
    ) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_constant_path_or_write_node(self, node);
    }

    fn visit_constant_path_and_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathAndWriteNode<'pr>,
    ) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_constant_path_and_write_node(self, node);
    }

    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'pr>) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_call_or_write_node(self, node);
    }

    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'pr>) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_call_and_write_node(self, node);
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
        ruby_prism::visit_index_or_write_node(self, node);
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        self.check_assignment_like_operator_spacing(&node.operator_loc());
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
                    Some(TrailingAnchor {
                        offset: node.location().start_offset(),
                        token_match: false,
                        exact_token: Some(node.location().as_slice().to_vec()),
                    }),
                    false,
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
