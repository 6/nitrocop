use std::collections::HashSet;

use ruby_prism::Visit;

use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;

/// Layout/RedundantLineBreak: Checks whether certain expressions that could fit
/// on a single line are broken up into multiple lines unnecessarily.
///
/// ## Implementation approach
/// Two-phase detection:
/// - **Phase 1 (AST)**: Visits CallNode and assignment write nodes. Uses walk-down
///   with `checked_chain_ranges` to approximate RuboCop's walk-up-to-outermost behavior.
/// - **Phase 2 (text)**: Detects backslash line continuations that could be collapsed.
///
/// ## Key differences from RuboCop
/// - RuboCop walks UP from `on_send` through parent sends, convertible blocks, and
///   binary operators to find the outermost expression. Nitrocop walks DOWN and uses
///   `checked_chain_ranges` + `part_of_reported_node` to approximate this.
/// - RuboCop's `configured_to_not_be_inspected?` only skips multiline blocks
///   (`any_descendant?(node, :any_block, &:multiline?)`). Nitrocop now matches this
///   by tracking multiline vs single-line blocks separately.
/// - RuboCop's `other_cop_takes_precedence?` is conditional on
///   `Layout/SingleLineBlockChain` being enabled. Nitrocop always checks for
///   single-line blocks in chains (slightly over-conservative, causing FNs).
///
/// ## Remaining gaps (FNs)
/// - No walk-up through `AndNode`/`OrNode` (binary operators) — standalone multiline
///   `&&`/`||` expressions without assignment are not checked.
/// - `contains_single_line_block` always fires (not conditional on SingleLineBlockChain
///   being enabled), causing FNs for single-line block patterns.
/// - No walk-up through convertible blocks (`method { ... }.chain`) — the block is not
///   merged with its send_node for length calculation.
///
/// ## Fixes applied (2026-03-09)
/// - Phase 2 now checks block and unsafe ranges before reporting backslash continuations.
/// - Added `ParenthesesNode` to unsafe ranges (maps to `:begin` in Parser AST).
/// - Fixed `too_long` method chain dot check to match RuboCop's `(?=(&)?\.\w)` regex.
/// - Split block range tracking into multiline-only (`contains_multiline_block`)
///   for more accurate InspectBlocks handling.
///
/// ## Fixes applied (2026-03-16)
/// - **Critical FP fix**: `UnsafeRangeCollector` now recurses into all node types
///   (DefNode, IfNode, CaseNode, etc.). Previously it stopped recursing when it hit
///   these nodes, so multiline strings/regexps/arrays nested inside methods or
///   conditionals were never collected as unsafe ranges. This caused ~thousands of FPs
///   in repos like slim-template (315 FPs from multiline %q{} strings inside def bodies).
/// - Added all missing operator/or/and write node visitors for instance variables,
///   class variables, global variables, constants, and constant paths (e.g.,
///   `@count += items.size`, `@@total += n`, `$var ||= compute`).
///
/// ## Fixes applied (2026-04-01)
/// - Fixed keyword-argument false negatives inside multiline chained block
///   expressions. Previously a multiline outer call like `items.each do ... end`
///   could mark its entire byte range as "checked" and suppress inner
///   multiline calls in the block body. The fix remains deliberately narrow:
///   contained calls are only unsuppressed for the validated keyword-hash
///   argument shape when they are the direct, sole statement inside a
///   multiline block body.
///
/// ## Fixes applied (2026-04-03)
/// - Extended that narrow block-body unsuppression to `map` blocks too.
///   RuboCop does flag direct multiline keyword-hash calls such as
///   `search_results.map do ... extract_uris(...) end`, but nitrocop still
///   excluded them because only `each`/`select`-style iterators were
///   allowlisted. Keeping the same structural guards and only adding `map`
///   recovers those FNs without broadening to unrelated block shapes.
///
/// ## Fixes applied (2026-04-04)
/// - **String value check**: `UnsafeRangeCollector` now uses
///   `StringNode::unescaped()` to check decoded string values for `\n`,
///   matching RuboCop's `safe_to_split?` which checks `n.value.include?("\n")`.
///   Previously only source-level newline bytes were checked, causing:
///   - FPs: strings with escape sequences like `"AT+CLAC\r\n"` were not marked
///     unsafe (source has no literal newline), so calls containing them were
///     incorrectly flagged.
///   - FNs: string concatenations with `\` line continuation like
///     `"foo" \ "bar"` were falsely marked unsafe (source spans lines), so
///     calls containing them were incorrectly suppressed.
/// - **Block body exclusion**: `checked_chain_ranges` now excludes block bodies.
///   In RuboCop, the send node's range does not include the block (the block is
///   a parent node), so calls inside block bodies are checked independently.
///   In Prism, `CallNode` includes the block, so the old code suppressed ALL
///   calls inside block bodies of chains. Now only the chain portion (up to the
///   block start) is marked as checked. Additionally, `part_of_checked_chain`
///   now checks whether a node is inside a block body within the chain range
///   (via `inside_block_body_within_chain`), so calls like `extract_uris(...)`
///   inside `search_results.map do ... end.flatten` are correctly unsuppressed
///   even when the outer chain's range encompasses the block body.
/// - **Non-convertible block handling**: When a `CallNode` has a block that is NOT
///   convertible (i.e., the call has arguments but no parentheses, like
///   `config.wrappers :default, ... do |b| ... end`), the offense check now uses
///   the "send-only" range (up to the block start) instead of the full range.
///   In RuboCop's Parser AST, `on_send` walks up through convertible blocks but
///   stops at non-convertible ones, checking only the send portion. This recovers
///   ~2,653 FNs and resolves ~616 FPs with zero regressions.
///
/// ## Fixes applied (2026-04-06)
/// - **Data structure nesting check**: `part_of_checked_chain` now uses
///   `is_nested_in_data_structure()` to allow calls nested inside hash, array,
///   or other data structures within a chain to be checked independently. In
///   RuboCop, `on_send` walks up only through parent `send_type?` nodes. Calls
///   inside hashes (`key: match_array([...])`), arrays, or splats have non-send
///   parents (pair, hash, array) that stop the walk-up, so they're checked
///   independently. Previously, nitrocop's byte-range-based `checked_chain_ranges`
///   suppressed ALL calls within a chain's range, including these nested calls.
///   Resolves ~1350 FNs (e.g., `match_array(...)` inside `have_attributes(...)`)
///   with zero regressions.
/// - **`too_long` backslash fix**: The combined-line length calculation now only
///   strips trailing line-continuation backslashes, not ALL backslash characters.
///   Previously, `combined.retain(|&b| b != b'\\')` removed content backslashes
///   like `\1_\2` in regex replacements and `\d` in character classes, making
///   the combined line appear shorter than it actually is. This caused FPs for
///   method chains containing regex patterns where the true combined length
///   exceeded 120 chars but the backslash-stripped version didn't. Resolves
///   ~594 FPs with zero regressions.
///
/// ## Fixes applied (2026-04-07)
/// - Added missing visitor handlers for `IndexOperatorWriteNode`, `IndexOrWriteNode`,
///   `IndexAndWriteNode`, `CallOperatorWriteNode`, `CallOrWriteNode`, and
///   `CallAndWriteNode`. Previously, index operator writes like `foo[bar] ||= value`
///   and call operator writes like `obj.method += value` were not visited, causing
///   FNs for these patterns. These handlers ensure the `check_assignment` method is
///   invoked for these node types, enabling detection of redundant line breaks in
///   index/call operator write expressions. Resolves ~2 FNs with zero regressions.
///
/// - NOTE: The CLI does not properly enable this preview cop even with `--preview`.
///   Unit tests bypass CLI filtering and work correctly.
pub struct RedundantLineBreak;

impl Cop for RedundantLineBreak {
    fn name(&self) -> &'static str {
        "Layout/RedundantLineBreak"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let inspect_blocks = config.get_bool("InspectBlocks", false);
        let max_line_length = config.get_usize("MaxLineLength", 120);

        // Collect comment line numbers (1-indexed) for the comment_within check.
        let comment_lines: HashSet<usize> = parse_result
            .comments()
            .map(|c| {
                let (line, _) = source.offset_to_line_col(c.location().start_offset());
                line
            })
            .collect();

        // Pre-collect ranges of unsafe-to-split constructs:
        // if/unless/case/begin/def nodes, heredocs, and multiline strings.
        let mut unsafe_collector = UnsafeRangeCollector { ranges: Vec::new() };
        unsafe_collector.visit(&parse_result.node());
        let unsafe_ranges = unsafe_collector.ranges;

        // Pre-collect block ranges (for InspectBlocks: false check)
        let mut block_collector = BlockRangeCollector {
            ranges: Vec::new(),
            source,
        };
        block_collector.visit(&parse_result.node());
        let block_ranges = block_collector.ranges;

        // Pre-collect single-line block ranges (for Layout/SingleLineBlockChain precedence)
        let mut sl_block_collector = SingleLineBlockCollector {
            ranges: Vec::new(),
            source,
            ancestors: Vec::new(),
        };
        sl_block_collector.visit(&parse_result.node());
        let single_line_block_ranges = sl_block_collector.ranges;

        // Phase 1: AST-based detection (method calls and assignments)
        let mut visitor = RedundantLineBreakVisitor {
            source,
            cop_name: self.name(),
            max_line_length,
            inspect_blocks,
            comment_lines: &comment_lines,
            unsafe_ranges: &unsafe_ranges,
            block_ranges: &block_ranges,
            single_line_block_ranges: &single_line_block_ranges,
            ast_diagnostics: Vec::new(),
            reported_starts: HashSet::new(),
            reported_ranges: Vec::new(),
            checked_chain_ranges: Vec::new(),
            ancestors: Vec::new(),
        };
        visitor.visit(&parse_result.node());

        let reported_starts = visitor.reported_starts;
        diagnostics.extend(visitor.ast_diagnostics);

        // Phase 2: Backslash continuation detection (existing text-based approach)
        check_backslash_continuations(
            self,
            source,
            code_map,
            max_line_length,
            inspect_blocks,
            diagnostics,
            &reported_starts,
            &unsafe_ranges,
            &block_ranges,
        );
    }
}

/// Collects byte ranges of unsafe-to-split constructs.
///
/// Matches RuboCop's `safe_to_split?` from `CheckSingleLineSuitability`:
///   node.each_descendant(:if, :case, :kwbegin, :any_def).none? &&
///     node.each_descendant(:dstr, :str).none? { |n| n.heredoc? || n.value.include?("\n") } &&
///     node.each_descendant(:begin, :sym).none? { |b| !b.single_line? }
///
/// Notably, RuboCop does NOT check for `:regexp` or array literals (`%w`, `%i`)
/// in `safe_to_split?`. Even though collapsing a multiline `/x` regex or `%w`
/// array changes semantics, RuboCop still flags them. We match that behavior
/// for corpus conformance.
struct UnsafeRangeCollector {
    /// (start_offset, end_offset) of nodes that make their parent unsafe to merge.
    ranges: Vec<(usize, usize)>,
}

impl<'pr> Visit<'pr> for UnsafeRangeCollector {
    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        // Recurse into children so nested unsafe constructs (strings, regexps,
        // inner ifs) inside the if body are also collected. The if itself is
        // unsafe for its parent, but children may need their own unsafe ranges
        // for inner assignments.
        ruby_prism::visit_if_node(self, node);
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_unless_node(self, node);
    }

    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_case_node(self, node);
    }

    fn visit_case_match_node(&mut self, node: &ruby_prism::CaseMatchNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_case_match_node(self, node);
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_begin_node(self, node);
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        // Must recurse: inner assignments need to see unsafe ranges from
        // strings, ifs, etc. nested inside this def body.
        ruby_prism::visit_def_node(self, node);
    }

    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if let Some(open) = node.opening_loc() {
            if open.as_slice().starts_with(b"<<") {
                let loc = node.location();
                self.ranges.push((loc.start_offset(), loc.end_offset()));
                return;
            }
        }
        // Check the decoded VALUE for newlines, not the source representation.
        // This matches RuboCop's `n.value.include?("\n")` in `safe_to_split?`.
        // Source-level checks miss escape sequences like "\r\n" (FPs) and
        // falsely catch backslash line continuations "foo" \ "bar" (FNs).
        if node.unescaped().contains(&b'\n') {
            let loc = node.location();
            self.ranges.push((loc.start_offset(), loc.end_offset()));
        }
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        if let Some(open) = node.opening_loc() {
            if open.as_slice().starts_with(b"<<") {
                let loc = node.location();
                self.ranges.push((loc.start_offset(), loc.end_offset()));
                return;
            }
        }
        // Don't check source-level newlines here. Backslash line continuations
        // ("foo #{x}" \ "bar #{y}") span source lines but the value has no \n.
        // Instead, rely on recursion into child StringNode parts, which use
        // unescaped() to correctly detect newlines in the decoded value.
        ruby_prism::visit_interpolated_string_node(self, node);
    }

    fn visit_symbol_node(&mut self, node: &ruby_prism::SymbolNode<'pr>) {
        // Check decoded value for newlines, matching RuboCop's safe_to_split?.
        if node.unescaped().contains(&b'\n') {
            let loc = node.location();
            self.ranges.push((loc.start_offset(), loc.end_offset()));
        }
    }

    fn visit_interpolated_symbol_node(&mut self, node: &ruby_prism::InterpolatedSymbolNode<'pr>) {
        // Rely on recursion into child StringNode parts for newline detection.
        ruby_prism::visit_interpolated_symbol_node(self, node);
    }

    /// Multiline parenthesized groups `(...)` — maps to `:begin` in Parser AST.
    /// RuboCop's `safe_to_split?` checks
    /// `node.each_descendant(:begin, :sym).none? { |b| !b.single_line? }`.
    fn visit_parentheses_node(&mut self, node: &ruby_prism::ParenthesesNode<'pr>) {
        let content = node.location().as_slice();
        if content.contains(&b'\n') {
            let loc = node.location();
            self.ranges.push((loc.start_offset(), loc.end_offset()));
        }
        // Still recurse into children to find nested unsafe constructs
        ruby_prism::visit_parentheses_node(self, node);
    }
}

/// Collects byte ranges of block/lambda nodes, tracking whether each is multiline.
struct BlockRangeCollector<'a> {
    /// (start_offset, end_offset, is_multiline)
    ranges: Vec<(usize, usize, bool)>,
    source: &'a SourceFile,
}

impl<'pr> Visit<'pr> for BlockRangeCollector<'_> {
    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        let loc = node.location();
        let (start_line, _) = self.source.offset_to_line_col(loc.start_offset());
        let (end_line, _) = self
            .source
            .offset_to_line_col(loc.end_offset().saturating_sub(1));
        let is_multiline = start_line != end_line;
        self.ranges
            .push((loc.start_offset(), loc.end_offset(), is_multiline));
        ruby_prism::visit_block_node(self, node);
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        let loc = node.location();
        let (start_line, _) = self.source.offset_to_line_col(loc.start_offset());
        let (end_line, _) = self
            .source
            .offset_to_line_col(loc.end_offset().saturating_sub(1));
        let is_multiline = start_line != end_line;
        self.ranges
            .push((loc.start_offset(), loc.end_offset(), is_multiline));
        ruby_prism::visit_lambda_node(self, node);
    }
}

/// Collects byte ranges of single-line block nodes whose parent (in Parser AST
/// terms) is a send with a dot.
///
/// Matches RuboCop's `other_cop_takes_precedence?` which checks:
///   `block_node.parent.send_type? && block_node.parent.loc.dot && !block_node.multiline?`
///
/// In Parser AST, `block_node.parent` is the CONTAINING node. For:
///   - `foo.map { ... }.compact` → block parent is `.compact` send (has dot) ✓
///   - `foo.bar(proc { ... })` → block parent is `.bar` send (has dot) ✓
///   - `x = foo.map { ... }` → block parent is assignment (no dot) ✗
///   - `bar(proc { ... })` → block parent is `bar` send (no dot) ✗
///
/// In Prism, a block is always a child of its "owning" CallNode. The "containing"
/// node in Parser AST terms is found by skipping the owning CallNode and any
/// ArgumentsNode wrapper in the ancestor stack.
struct SingleLineBlockCollector<'a, 'pr> {
    ranges: Vec<(usize, usize)>,
    source: &'a SourceFile,
    ancestors: Vec<ruby_prism::Node<'pr>>,
}

impl<'pr> Visit<'pr> for SingleLineBlockCollector<'_, 'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {}

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        let loc = node.location();
        let (start_line, _) = self.source.offset_to_line_col(loc.start_offset());
        let (end_line, _) = self
            .source
            .offset_to_line_col(loc.end_offset().saturating_sub(1));
        if start_line == end_line && self.containing_call_has_dot() {
            self.ranges.push((loc.start_offset(), loc.end_offset()));
        }
        ruby_prism::visit_block_node(self, node);
    }
}

impl SingleLineBlockCollector<'_, '_> {
    /// Check if the block's "parent" in Parser AST terms is a CallNode with a dot.
    ///
    /// Walk up ancestors, skipping the owning CallNode (immediate parent of the
    /// block in Prism) and any ArgumentsNode wrapper (Prism-only, no Parser
    /// equivalent). The next significant node is the "containing" context.
    fn containing_call_has_dot(&self) -> bool {
        let len = self.ancestors.len();
        // Need at least 2 ancestors: the BlockNode itself (pushed by
        // visit_branch_node_enter before visit_block_node runs) and
        // the owning CallNode above it.
        if len < 2 {
            return false;
        }
        let mut skipped_owning_call = false;
        // Start at len-2 to skip the current BlockNode at the top of the stack.
        for i in (0..len - 1).rev() {
            let ancestor = &self.ancestors[i];
            // The first CallNode we encounter (nearest) is the "owning" call
            // (e.g., `proc` for `proc { ... }`, or `.map` for `.map { ... }`).
            // Skip it to find the containing context.
            if !skipped_owning_call {
                if ancestor.as_call_node().is_some() {
                    skipped_owning_call = true;
                    continue;
                }
            }
            // ArgumentsNode is a Prism wrapper with no Parser AST equivalent.
            if ancestor.as_arguments_node().is_some() {
                continue;
            }
            // Found the containing node. Check if it's a CallNode with a dot.
            return ancestor
                .as_call_node()
                .is_some_and(|c| c.call_operator_loc().is_some());
        }
        false
    }
}

/// AST visitor that finds multiline expressions that could fit on a single line.
struct RedundantLineBreakVisitor<'a, 'pr> {
    source: &'a SourceFile,
    cop_name: &'static str,
    max_line_length: usize,
    inspect_blocks: bool,
    comment_lines: &'a HashSet<usize>,
    unsafe_ranges: &'a [(usize, usize)],
    block_ranges: &'a [(usize, usize, bool)],
    single_line_block_ranges: &'a [(usize, usize)],
    ast_diagnostics: Vec<Diagnostic>,
    reported_starts: HashSet<usize>,
    /// Byte ranges of nodes already reported, to skip descendant checks.
    reported_ranges: Vec<(usize, usize)>,
    /// Byte ranges of outermost call chain nodes that were checked (whether reported or not).
    /// Inner CallNodes within these ranges are skipped to match RuboCop's walk-up behavior.
    checked_chain_ranges: Vec<(usize, usize)>,
    ancestors: Vec<ruby_prism::Node<'pr>>,
}

impl<'a, 'pr> RedundantLineBreakVisitor<'a, 'pr> {
    fn is_multiline(&self, start_offset: usize, end_offset: usize) -> bool {
        let (start_line, _) = self.source.offset_to_line_col(start_offset);
        let (end_line, _) = self
            .source
            .offset_to_line_col(end_offset.saturating_sub(1).max(start_offset));
        start_line != end_line
    }

    /// Check if combining lines of this span would exceed max_line_length.
    fn too_long(&self, start_offset: usize, end_offset: usize) -> bool {
        let (start_line, _) = self.source.offset_to_line_col(start_offset);
        let (end_line, _) = self
            .source
            .offset_to_line_col(end_offset.saturating_sub(1).max(start_offset));

        let lines: Vec<&[u8]> = self.source.lines().collect();
        let mut combined = Vec::new();
        for line_num in start_line..=end_line {
            if line_num > lines.len() {
                break;
            }
            let line = lines[line_num - 1];
            // Strip trailing whitespace, then strip trailing backslash (line continuation).
            // Only remove the line-continuation backslash, NOT backslashes that are part of
            // content (e.g., \1_\2 in regex replacements, \d in character classes).
            let trimmed_end = trim_trailing_whitespace(line);
            let without_continuation = if trimmed_end.ends_with(b"\\") {
                trim_trailing_whitespace(&trimmed_end[..trimmed_end.len() - 1])
            } else {
                trimmed_end
            };
            if combined.is_empty() {
                combined.extend_from_slice(without_continuation);
            } else {
                let trimmed = trim_leading_whitespace(without_continuation);
                if starts_with_method_chain_dot(trimmed) {
                    combined.extend_from_slice(trimmed);
                } else {
                    combined.push(b' ');
                    combined.extend_from_slice(trimmed);
                }
            }
        }

        combined.len() > self.max_line_length
    }

    fn comment_within(&self, start_offset: usize, end_offset: usize) -> bool {
        let (start_line, _) = self.source.offset_to_line_col(start_offset);
        let (end_line, _) = self
            .source
            .offset_to_line_col(end_offset.saturating_sub(1).max(start_offset));
        self.comment_lines
            .iter()
            .any(|&line| line >= start_line && line <= end_line)
    }

    /// Check if any unsafe range is contained within (or overlaps) the given span.
    fn contains_unsafe(&self, start_offset: usize, end_offset: usize) -> bool {
        self.unsafe_ranges
            .iter()
            .any(|&(us, ue)| us >= start_offset && ue <= end_offset)
    }

    /// Check if any multiline block range is contained within the given span.
    /// This matches RuboCop's `any_descendant?(node, :any_block, &:multiline?)`.
    fn contains_multiline_block(&self, start_offset: usize, end_offset: usize) -> bool {
        self.block_ranges
            .iter()
            .any(|&(bs, be, multiline)| multiline && bs >= start_offset && be <= end_offset)
    }

    /// Check if any single-line block is contained within the given span.
    fn contains_single_line_block(&self, start_offset: usize, end_offset: usize) -> bool {
        self.single_line_block_ranges
            .iter()
            .any(|&(bs, be)| bs >= start_offset && be <= end_offset)
    }

    fn suitable_as_single_line(&self, start_offset: usize, end_offset: usize) -> bool {
        !self.too_long(start_offset, end_offset)
            && !self.comment_within(start_offset, end_offset)
            && !self.contains_unsafe(start_offset, end_offset)
    }

    fn configured_to_not_be_inspected(&self, start_offset: usize, end_offset: usize) -> bool {
        // Layout/SingleLineBlockChain takes precedence for single-line blocks in chains
        // TODO: This should be conditional on Layout/SingleLineBlockChain being enabled,
        // matching RuboCop's single_line_block_chain_enabled? check.
        if self.contains_single_line_block(start_offset, end_offset) {
            return true;
        }
        // When InspectBlocks is false (default), skip expressions containing
        // multiline blocks. This matches RuboCop's:
        //   node.any_block_type? || any_descendant?(node, :any_block, &:multiline?)
        if !self.inspect_blocks && self.contains_multiline_block(start_offset, end_offset) {
            return true;
        }
        false
    }

    /// Check if a byte offset falls within any already-reported node's range.
    fn part_of_reported_node(&self, start_offset: usize, end_offset: usize) -> bool {
        self.reported_ranges
            .iter()
            .any(|&(rs, re)| start_offset >= rs && end_offset <= re)
    }

    /// Check if a node is an inner part of a call chain that was already checked.
    /// This prevents inner CallNodes from being individually checked when the
    /// outermost CallNode in the chain was already visited (and either reported or rejected).
    /// Calls inside block bodies within the chain are NOT suppressed — in RuboCop's
    /// Parser AST, blocks are parent nodes (not part of the send's range), so calls
    /// inside block bodies are checked independently.
    /// Calls nested inside data structures (hashes, arrays, etc.) within arguments
    /// are also NOT suppressed — in RuboCop's `on_send`, the walk-up only follows
    /// parent send_type? nodes, so calls inside hash/array structures don't walk up
    /// and are checked independently.
    fn part_of_checked_chain(&self, start_offset: usize, end_offset: usize) -> bool {
        self.checked_chain_ranges.iter().any(|&(cs, ce)| {
            start_offset >= cs
                && end_offset <= ce
                && (start_offset > cs || end_offset < ce)
                && !self.inside_block_body_within_chain(start_offset, end_offset, cs, ce)
                && !self.is_nested_in_data_structure()
        })
    }

    /// Returns true if the current call is nested inside a data structure (hash, array,
    /// assoc pair, etc.) relative to its nearest ancestor CallNode. In RuboCop's Parser
    /// AST, such nesting breaks the walk-up in `on_send` because the intermediate nodes
    /// (hash, pair, array) are not `send_type?`. In Prism, ArgumentsNode is the only
    /// intermediate wrapper that doesn't break the walk-up (it has no Parser equivalent).
    fn is_nested_in_data_structure(&self) -> bool {
        // ancestors includes the current node as the last element.
        // Walk up from the second-to-last to find the nearest ancestor CallNode.
        if self.ancestors.len() < 2 {
            return false;
        }
        for i in (0..self.ancestors.len() - 1).rev() {
            let ancestor = &self.ancestors[i];
            // If we reach a CallNode, this node is a direct child (receiver or argument)
            // — not nested in a data structure.
            if ancestor.as_call_node().is_some() {
                return false;
            }
            // ArgumentsNode is Prism's wrapper for arguments; it doesn't exist in Parser
            // AST, so skip it (it doesn't break the walk-up).
            if ancestor.as_arguments_node().is_some() {
                continue;
            }
            // Any other node type (KeywordHashNode, HashNode, ArrayNode, AssocNode,
            // SplatNode, ParenthesesNode, etc.) breaks the walk-up in RuboCop.
            return true;
        }
        false
    }

    /// Returns true if the node at (start, end) is inside a block body that
    /// itself is contained within the checked chain range (cs, ce).
    fn inside_block_body_within_chain(
        &self,
        start: usize,
        end: usize,
        chain_start: usize,
        chain_end: usize,
    ) -> bool {
        self.block_ranges
            .iter()
            .any(|&(bs, be, _)| bs >= chain_start && be <= chain_end && start >= bs && end <= be)
    }

    fn register_offense(&mut self, start_offset: usize, end_offset: usize) {
        let (line, col) = self.source.offset_to_line_col(start_offset);

        if self.reported_starts.contains(&line) {
            return;
        }
        self.reported_starts.insert(line);
        self.reported_ranges.push((start_offset, end_offset));

        self.ast_diagnostics.push(Diagnostic {
            path: self.source.path_str().to_string(),
            location: crate::diagnostic::Location { line, column: col },
            severity: crate::diagnostic::Severity::Convention,
            cop_name: self.cop_name.to_string(),
            message: "Redundant line break detected.".to_string(),
            corrected: false,
        });
    }
}

impl<'pr> Visit<'pr> for RedundantLineBreakVisitor<'_, 'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {}

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let loc = node.location();
        let start_offset = loc.start_offset();
        let end_offset = loc.end_offset();

        // In RuboCop's Parser AST, the block is a parent node of the send.
        // In Prism, the CallNode includes the block. For offense checks, use
        // the "send-only" range (excluding the block) when the block is NOT
        // convertible. A block is convertible when the send is parenthesized
        // or has no args — in that case, RuboCop's walk-up merges them.
        let has_non_convertible_block =
            node.block()
                .and_then(|b| b.as_block_node())
                .is_some_and(|_| {
                    // Not convertible: has arguments AND is not parenthesized
                    node.arguments().is_some() && node.opening_loc().is_none()
                });
        let check_end = if has_non_convertible_block {
            node.block()
                .and_then(|b| b.as_block_node())
                .map_or(end_offset, |block| block.location().start_offset())
        } else {
            end_offset
        };

        let skip_for_checked_chain = self.part_of_checked_chain(start_offset, end_offset);

        if self.is_multiline(start_offset, check_end)
            && !self.part_of_reported_node(start_offset, end_offset)
            && !skip_for_checked_chain
        {
            // This is the outermost multiline CallNode in its chain (since we
            // visit top-down and inner calls would be caught by part_of_checked_chain).
            // Record it so inner CallNodes in the chain are skipped, matching
            // RuboCop's walk-up-to-outermost behavior.
            let has_call_receiver = node.receiver().and_then(|r| r.as_call_node()).is_some();
            if has_call_receiver {
                // This node has a call chain underneath. Mark the chain range
                // so inner calls are not individually checked.
                // Exclude block bodies: in RuboCop, the send node's range does
                // not include the block (the block is a parent node). Calls
                // inside block bodies should be checked independently.
                let effective_end = node
                    .block()
                    .and_then(|b| b.as_block_node())
                    .map_or(end_offset, |block| block.location().start_offset());
                self.checked_chain_ranges
                    .push((start_offset, effective_end));
            }

            // Skip index access chains: hash[:foo][:bar]
            let is_index_chain = if node.name().as_slice() == b"[]" {
                node.receiver()
                    .and_then(|r| r.as_call_node())
                    .is_some_and(|r| r.name().as_slice() == b"[]")
            } else {
                false
            };

            // When InspectBlocks is false and this CallNode has a convertible block,
            // the node maps to a block_type in RuboCop's walk-up (on_send walks
            // through convertible blocks, making the outermost node a :block).
            // RuboCop's `node.any_block_type?` returns true → skip.
            // A block is convertible when: parenthesized OR no explicit args.
            let has_convertible_block = !has_non_convertible_block
                && node.block().and_then(|b| b.as_block_node()).is_some();

            if !is_index_chain
                && self.suitable_as_single_line(start_offset, check_end)
                && !self.configured_to_not_be_inspected(start_offset, check_end)
                && !(has_convertible_block && !self.inspect_blocks)
            {
                self.register_offense(start_offset, check_end);
            }
        }

        ruby_prism::visit_call_node(self, node);
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_instance_variable_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_instance_variable_write_node(self, node);
    }

    fn visit_class_variable_write_node(&mut self, node: &ruby_prism::ClassVariableWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_class_variable_write_node(self, node);
    }

    fn visit_global_variable_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_global_variable_write_node(self, node);
    }

    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_constant_write_node(self, node);
    }

    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_constant_path_write_node(self, node);
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_multi_write_node(self, node);
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_local_variable_operator_write_node(self, node);
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_local_variable_or_write_node(self, node);
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_local_variable_and_write_node(self, node);
    }

    fn visit_instance_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOperatorWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_instance_variable_operator_write_node(self, node);
    }

    fn visit_instance_variable_or_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOrWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_instance_variable_or_write_node(self, node);
    }

    fn visit_instance_variable_and_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableAndWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_instance_variable_and_write_node(self, node);
    }

    fn visit_class_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOperatorWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_class_variable_operator_write_node(self, node);
    }

    fn visit_class_variable_or_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOrWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_class_variable_or_write_node(self, node);
    }

    fn visit_class_variable_and_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableAndWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_class_variable_and_write_node(self, node);
    }

    fn visit_global_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOperatorWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_global_variable_operator_write_node(self, node);
    }

    fn visit_global_variable_or_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOrWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_global_variable_or_write_node(self, node);
    }

    fn visit_global_variable_and_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableAndWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_global_variable_and_write_node(self, node);
    }

    fn visit_constant_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantOperatorWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_constant_operator_write_node(self, node);
    }

    fn visit_constant_or_write_node(&mut self, node: &ruby_prism::ConstantOrWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_constant_or_write_node(self, node);
    }

    fn visit_constant_and_write_node(&mut self, node: &ruby_prism::ConstantAndWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_constant_and_write_node(self, node);
    }

    fn visit_constant_path_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOperatorWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_constant_path_operator_write_node(self, node);
    }

    fn visit_constant_path_or_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOrWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_constant_path_or_write_node(self, node);
    }

    fn visit_constant_path_and_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathAndWriteNode<'pr>,
    ) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_constant_path_and_write_node(self, node);
    }

    fn visit_call_operator_write_node(&mut self, node: &ruby_prism::CallOperatorWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_call_operator_write_node(self, node);
    }

    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_call_or_write_node(self, node);
    }

    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_call_and_write_node(self, node);
    }

    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_index_operator_write_node(self, node);
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_index_or_write_node(self, node);
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        let loc = node.location();
        self.check_assignment(loc.start_offset(), loc.end_offset());
        ruby_prism::visit_index_and_write_node(self, node);
    }
}

impl RedundantLineBreakVisitor<'_, '_> {
    fn check_assignment(&mut self, start_offset: usize, end_offset: usize) {
        if !self.is_multiline(start_offset, end_offset) {
            return;
        }
        if self.part_of_reported_node(start_offset, end_offset) {
            return;
        }
        if !self.suitable_as_single_line(start_offset, end_offset) {
            return;
        }
        if self.configured_to_not_be_inspected(start_offset, end_offset) {
            return;
        }
        self.register_offense(start_offset, end_offset);
    }
}

/// Phase 2: backslash continuation detection (text-based).
#[allow(clippy::too_many_arguments)]
fn check_backslash_continuations(
    cop: &RedundantLineBreak,
    source: &SourceFile,
    code_map: &CodeMap,
    max_line_length: usize,
    inspect_blocks: bool,
    diagnostics: &mut Vec<Diagnostic>,
    already_reported: &HashSet<usize>,
    unsafe_ranges: &[(usize, usize)],
    block_ranges: &[(usize, usize, bool)],
) {
    let content = source.as_bytes();
    let lines: Vec<&[u8]> = source.lines().collect();

    let mut line_starts: Vec<usize> = Vec::with_capacity(lines.len());
    let mut offset = 0usize;
    for (i, line) in lines.iter().enumerate() {
        line_starts.push(offset);
        offset += line.len();
        if i < lines.len() - 1 || (offset < content.len() && content[offset] == b'\n') {
            offset += 1;
        }
    }

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = trim_trailing_whitespace(line);

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        if !trimmed.ends_with(b"\\") || i + 1 >= lines.len() {
            i += 1;
            continue;
        }

        let trimmed_content = trim_leading_whitespace(trimmed);
        if trimmed_content.starts_with(b"#") {
            i += 1;
            continue;
        }

        let backslash_offset = line_starts[i] + trimmed.len() - 1;
        if !code_map.is_code(backslash_offset) {
            i += 1;
            continue;
        }

        let group_start = i;
        let mut group_end = i;
        while group_end + 1 < lines.len() {
            let t = trim_trailing_whitespace(lines[group_end]);
            if !t.ends_with(b"\\") {
                break;
            }
            let next_trimmed_content =
                trim_leading_whitespace(trim_trailing_whitespace(lines[group_end + 1]));
            if next_trimmed_content.starts_with(b"#") {
                break;
            }
            group_end += 1;
        }
        let final_line_idx = group_end + 1;
        if final_line_idx >= lines.len() {
            i = final_line_idx;
            continue;
        }

        let report_line = group_start + 1; // 1-indexed
        if already_reported.contains(&report_line) {
            i = final_line_idx + 1;
            continue;
        }

        // Check if the backslash group's byte range overlaps with any unsafe
        // construct (if/case/begin/def/heredoc/multiline-string) or block.
        // This matches RuboCop's AST-level checks that prevent collapsing
        // expressions containing these constructs.
        let group_byte_start = line_starts[group_start];
        let group_byte_end = if final_line_idx < line_starts.len() {
            line_starts[final_line_idx] + lines[final_line_idx].len()
        } else {
            content.len()
        };

        let has_unsafe = unsafe_ranges
            .iter()
            .any(|&(us, ue)| us >= group_byte_start && ue <= group_byte_end);
        if has_unsafe {
            i = final_line_idx + 1;
            continue;
        }

        // When InspectBlocks is false (default), skip backslash groups that
        // overlap with any block (single-line or multiline). This is slightly
        // more conservative than RuboCop's AST-level check, but prevents Phase 2
        // from flagging expressions that the AST phase would handle differently.
        if !inspect_blocks {
            let has_block = block_ranges
                .iter()
                .any(|&(bs, be, _)| bs < group_byte_end && be > group_byte_start);
            if has_block {
                i = final_line_idx + 1;
                continue;
            }
        }

        // Build the combined single-line version.
        let indent = leading_whitespace_len(lines[group_start]);
        let mut combined = Vec::new();
        combined.extend_from_slice(&lines[group_start][..indent]);

        for (j, bline) in lines[group_start..=group_end].iter().enumerate() {
            let t = trim_trailing_whitespace(bline);
            if t.is_empty() {
                continue;
            }
            let before_bs = trim_trailing_whitespace(&t[..t.len() - 1]);
            let content_part = trim_leading_whitespace(before_bs);

            if j == 0 {
                combined.extend_from_slice(content_part);
            } else {
                combined.push(b' ');
                combined.extend_from_slice(content_part);
            }
        }

        let final_content = trim_leading_whitespace(lines[final_line_idx]);
        if !final_content.is_empty() {
            combined.push(b' ');
            combined.extend_from_slice(trim_trailing_whitespace(final_content));
        }

        if combined.len() > max_line_length {
            i = final_line_idx + 1;
            continue;
        }

        let next_content = trim_leading_whitespace(lines[group_start + 1]);
        if next_content.starts_with(b"&&") || next_content.starts_with(b"||") {
            i = final_line_idx + 1;
            continue;
        }

        if is_string_concat_continuation(&lines, group_start, group_end) {
            i = final_line_idx + 1;
            continue;
        }

        diagnostics.push(cop.diagnostic(
            source,
            report_line,
            0,
            "Redundant line break detected.".to_string(),
        ));

        i = final_line_idx + 1;
    }
}

fn trim_trailing_whitespace(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && (line[end - 1] == b' ' || line[end - 1] == b'\t') {
        end -= 1;
    }
    &line[..end]
}

fn trim_leading_whitespace(line: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < line.len() && (line[start] == b' ' || line[start] == b'\t') {
        start += 1;
    }
    &line[start..]
}

fn is_string_concat_continuation(lines: &[&[u8]], group_start: usize, group_end: usize) -> bool {
    for j in group_start..group_end {
        let t = trim_trailing_whitespace(lines[j]);
        if t.is_empty() || t[t.len() - 1] != b'\\' {
            return false;
        }
        let before_bs = trim_trailing_whitespace(&t[..t.len() - 1]);
        if before_bs.is_empty() {
            return false;
        }
        let last_char = before_bs[before_bs.len() - 1];
        if last_char != b'\'' && last_char != b'"' {
            return false;
        }

        if j + 1 < lines.len() {
            let next_content = trim_leading_whitespace(lines[j + 1]);
            if next_content.is_empty() {
                return false;
            }
            let first_char = next_content[0];
            if first_char != b'\'' && first_char != b'"' {
                return false;
            }
        }
    }
    if group_end < lines.len() {
        let tail_content = trim_leading_whitespace(lines[group_end]);
        if tail_content.is_empty() {
            return false;
        }
        let first_char = tail_content[0];
        if first_char != b'\'' && first_char != b'"' {
            return false;
        }
    }
    true
}

/// Check if a trimmed line starts with a method chain dot followed by a word
/// character, matching RuboCop's `/\n\s*(?=(&)?\.\w)/` pattern.
/// Lines starting with `.operator` (like `.[]`, `.==`, `.+`) get a space
/// when joining, while `.method_name` chains get no space.
fn starts_with_method_chain_dot(trimmed: &[u8]) -> bool {
    if trimmed.starts_with(b"&.") {
        trimmed.len() > 2 && is_word_char(trimmed[2])
    } else if trimmed.starts_with(b".") {
        trimmed.len() > 1 && is_word_char(trimmed[1])
    } else {
        false
    }
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn leading_whitespace_len(line: &[u8]) -> usize {
    let mut count = 0;
    for &b in line {
        if b == b' ' || b == b'\t' {
            count += 1;
        } else {
            break;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(RedundantLineBreak, "cops/layout/redundant_line_break");
}
