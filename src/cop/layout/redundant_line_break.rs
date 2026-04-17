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
///   `Layout/SingleLineBlockChain` being enabled. Nitrocop now mirrors that by
///   using an injected `SingleLineBlockChainEnabled` flag from config.
///
/// ## Remaining gaps (FNs)
/// - No walk-up through `AndNode`/`OrNode` (binary operators) — standalone multiline
///   `&&`/`||` expressions without assignment are not checked.
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
/// ## Fixes applied (2026-04-08)
/// - **Direct argument walk-up**: nested multiline calls used as direct arguments
///   of an outer multiline call are now suppressed when the outer call has already
///   been checked. RuboCop's `on_send` walks from the inner send to its parent send
///   even when that parent relation comes from an argument position, not only a
///   receiver chain. Nitrocop previously only tracked receiver chains, which caused
///   false positives for cases like `SeedDump.dump(EventInstance\n.where(...), ...)`
///   and similar backslash-heavy argument lists.
/// - **Multiline regexp safety**: Prism represents a plain regexp as a single
///   `RegularExpressionNode`, but Parser exposes newline-containing regexp bodies
///   through descendant `:str` nodes. RuboCop therefore treats multiline regexps as
///   unsafe in `safe_to_split?`. Nitrocop now explicitly marks multiline regexp
///   literals unsafe so assignments like `GROUPED_INPUT_PATTERN = /.../x.freeze`
///   are no longer falsely flagged.
///
/// ## Fixes applied (2026-04-08, second batch)
/// - **Binary operator walk-up**: RuboCop's `on_send` walks up through parent
///   `OrNode`/`AndNode` (both `||`/`&&` and `or`/`and`). After walking up,
///   `operator_keyword?` returns true for these nodes, and `require_backslash?`
///   gates the offense on the operator line ending with `\`. Without `\`, no
///   offense is registered on ANY send inside the expression. Nitrocop did not
///   perform this walk-up, so multiline calls that were the RHS of `||`/`&&`
///   (e.g., `destroy || raise(...)`, `must_not_cache? || stale?(...)`) were
///   incorrectly flagged. The fix adds `inside_binary_op_without_backslash()`
///   which walks up the ancestor stack following RuboCop's walk-up rules and
///   suppresses offenses when the binary operator line lacks a trailing `\`.
///   Resolves ~144 FPs and ~51 FNs with zero regressions.
///
/// ## Fixes applied (2026-04-09)
/// - **Trailing `&.` join length**: RuboCop's single-line suitability
///   check collapses safe-navigation chains split after a trailing `&.` like
///   `foo&.\n  bar` without inserting a space. Nitrocop only handled the
///   leading-dot form (`foo\n  .bar`), so it overestimated joined length for
///   these safe-navigation chains and skipped real offenses when the true
///   joined line fit under the configured maximum.
/// - **Unary `!` wrapper anchoring**: Prism exposes `!foo&.\n  bar` as an
///   outer unary-`!` call wrapped around the multiline send, but RuboCop
///   reports the underlying send start (`foo`, not `!`). Nitrocop now skips
///   that wrapper so the inner call is checked and anchored like RuboCop.
/// - **Unary `!` wrapper narrowing**: that skip only applies to safe-navigation
///   receiver chains. RuboCop still reports ordinary unary-negated multiline
///   sends such as `!foo.\n  bar` and `!checks.values.\n  find { ... }`, but
///   nitrocop was skipping every unary-`!` wrapper and missing those offenses.
///
/// ## Fixes applied (2026-04-09, second batch)
/// - **UTF-8 character counting**: `too_long()` and Phase 2's combined-line check
///   now measure character length instead of byte length, matching RuboCop's
///   `String#length` which counts characters. Previously, multi-byte characters
///   (CJK, accented, etc.) inflated the measured length, causing FNs for lines
///   that fit within 120 characters but exceeded 120 bytes. Fixed by adding
///   `utf8_char_count()` which counts non-continuation bytes. Resolves ~92 FNs
///   (e.g., BCDice repo with Japanese text).
/// - **Phase 2 unsafe range overlap**: Phase 2's `has_unsafe` check now detects
///   unsafe ranges that START within the backslash group but extend beyond it
///   (e.g., case/until/while expressions on the continuation line). Previously
///   only ranges fully contained within the group were detected, causing FPs for
///   patterns like `foo || \ case @mode ... end` and `parent \ until cond`.
///   Resolves ~41 FPs (e.g., ruby2js repo). Added `UntilNode`, `WhileNode`, and
///   `ForNode` visitors to `UnsafeRangeCollector` to support modifier keywords.
///
/// ## Fixes applied (2026-04-10)
/// - **String continuation merging in `too_long`**: RuboCop's `to_single_line`
///   merges adjacent string literals across backslash continuations:
///   `"foo" \ "bar"` → `"foobar"` (same quotes), `"foo" \ 'bar'` → `"foo" + 'bar'`
///   (different quotes). Nitrocop's `too_long` previously joined these lines with
///   a space, keeping both sets of quotes: `"foo" "bar"` — 2 extra characters per
///   continuation. For deeply-indented expressions with string continuations, this
///   caused the combined length to exceed 120 chars when RuboCop's version fit,
///   resulting in FNs. For example, `raise ArgumentError, "long..." \ "msg"` inside
///   a 4-level-deep nesting would measure 124 chars (nitrocop) vs 117 chars (RuboCop).
///   Added `merge_string_continuation()` helper and `prev_had_backslash` tracking in
///   `too_long` to match RuboCop's merging behavior.
/// - **Phase 2 comma-tail span**: the text-based backslash pass now keeps walking
///   through immediately-following comma-terminated lines before measuring length.
///   RuboCop judges the whole continued call (for example `attr_reader \` followed
///   by many symbol arguments), but nitrocop previously only joined the backslash
///   line with the very next line. That produced false positives for long DSL-style
///   argument lists that still obviously continued after the first continuation line.
///
/// ## Fixes applied (2026-04-11)
/// - **Non-convertible block chain suppression**: outer sends such as
///   `expect(...).to receive(:find) do ... end.and_return(...)` were still
///   marking their full Prism byte range as "checked", which suppressed the
///   inner send that actually owns the non-convertible block. RuboCop stops its
///   `on_send` walk-up at that block boundary, so the multiline `expect(` /
///   `expect_any_instance_of(...).` send is the real offense target. Nitrocop
///   now lets call nodes that own a non-convertible block bypass outer
///   `checked_chain_ranges` suppression so those RSpec chains are checked at the
///   same boundary RuboCop uses.
///
/// ## Fixes applied (2026-04-12)
/// - **Phase 2 comment guard**: backslash continuations with inline comments in
///   the continued lines (for example `attr_reader \` lists with trailing
///   `# DEV(...)` comments) are now skipped. RuboCop will not collapse those
///   comment-bearing expressions onto one line.
/// - **Phase 2 branch-tail string continuations**: backslash string
///   continuations that end directly before `else`/`elsif`/`when`/`rescue`/`end`
///   are skipped without suppressing unrelated nested expressions. This matches
///   RuboCop for string continuations inside `if ... else ... end` assignments
///   while avoiding the broad FN regression from suppressing whole enclosing
///   conditional ranges.
/// - **Multiline interpolated string safety**: interpolated strings that contain
///   literal newlines are now marked unsafe-to-split as whole nodes, except for
///   `%`-newline delimiters like `x = %\n"#{foo}"` where the delimiter newline is
///   not part of the string's effective value. RuboCop treats expressions like
///   `\"<h2>#{\n  call\n}</h2>\"` as unsafe, but still flags newline-delimited
///   percent strings that fit on one line.
/// - **Safe-navigation block-chain precedence**: `Layout/SingleLineBlockChain`
///   only takes precedence for ordinary `.` chains, not `&.` chains. Nitrocop
///   now matches that by excluding safe-navigation callers from the
///   single-line-block precedence collector, so patterns like
///   `registry\n  .find { ... }\n  &.command_class` are correctly reported.
/// - **Phase 2 enclosing-expression guard**: the text fallback now skips
///   backslash groups that are already covered by a larger multiline AST call
///   chain, or by a class-header inheritance span like `class Foo < \ Bar`.
///   RuboCop judges those larger expressions as a whole, so nitrocop must not
///   emit extra inner reports for long operator chains or superclass headers.
///
/// ## Fixes applied (2026-04-17)
/// - **SingleLineBlockChain gating**: `configured_to_not_be_inspected()` now
///   only defers to `Layout/SingleLineBlockChain` when that cop is actually
///   enabled in the resolved config. Previously the precedence check was always
///   active, so repos that disabled `Layout/SingleLineBlockChain` still had
///   multiline chains like `e.select { ... }\n  .join` suppressed here, causing
///   false negatives relative to RuboCop.
/// - **Stabby lambda precedence**: RuboCop's
///   `other_cop_takes_precedence?` also considers single-line stabby lambdas
///   (`-> { ... }`) whose containing send has a dot. Nitrocop only tracked
///   `BlockNode`, so multiline dotted calls like
///   `assoc.has_many ..., -> { ... }, ...` and outer wrappers like
///   `assert_equal(..., obj.call(-> { ... }))` were incorrectly flagged even
///   though `Layout/SingleLineBlockChain` should take precedence.
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
        let single_line_block_chain_enabled = config.get_bool("SingleLineBlockChainEnabled", true);

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
        let mut unsafe_collector = UnsafeRangeCollector {
            ranges: Vec::new(),
            group_blocking_ranges: Vec::new(),
        };
        unsafe_collector.visit(&parse_result.node());
        let unsafe_ranges = unsafe_collector.ranges;
        let group_blocking_ranges = unsafe_collector.group_blocking_ranges;

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
            single_line_block_chain_enabled,
            ast_diagnostics: Vec::new(),
            reported_starts: HashSet::new(),
            reported_ranges: Vec::new(),
            checked_chain_ranges: Vec::new(),
            ancestors: Vec::new(),
        };
        visitor.visit(&parse_result.node());

        let RedundantLineBreakVisitor {
            reported_starts,
            ast_diagnostics,
            checked_chain_ranges,
            ..
        } = visitor;
        diagnostics.extend(ast_diagnostics);

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
            &group_blocking_ranges,
            &checked_chain_ranges,
            &block_ranges,
            &comment_lines,
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
/// Parser exposes multiline regexp bodies through descendant `:str` nodes, so
/// RuboCop's `safe_to_split?` implicitly treats multiline regexps as unsafe.
/// Prism represents a plain regexp as a single `RegularExpressionNode`, so
/// nitrocop must explicitly mark those ranges unsafe. Arrays (`%w`, `%i`) are
/// still intentionally left alone because RuboCop does flag those.
struct UnsafeRangeCollector {
    /// (start_offset, end_offset) of nodes that make their parent unsafe to merge.
    ranges: Vec<(usize, usize)>,
    /// Expression ranges that should also suppress Phase 2 backslash groups
    /// when they cover the whole group.
    group_blocking_ranges: Vec<(usize, usize)>,
}

impl<'pr> Visit<'pr> for UnsafeRangeCollector {
    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        self.group_blocking_ranges
            .push((loc.start_offset(), loc.end_offset()));
        // Recurse into children so nested unsafe constructs (strings, regexps,
        // inner ifs) inside the if body are also collected. The if itself is
        // unsafe for its parent, but children may need their own unsafe ranges
        // for inner assignments.
        ruby_prism::visit_if_node(self, node);
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        self.group_blocking_ranges
            .push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_unless_node(self, node);
    }

    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        self.group_blocking_ranges
            .push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_case_node(self, node);
    }

    fn visit_case_match_node(&mut self, node: &ruby_prism::CaseMatchNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        self.group_blocking_ranges
            .push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_case_match_node(self, node);
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        self.group_blocking_ranges
            .push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_begin_node(self, node);
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        // Must recurse: inner assignments need to see unsafe ranges from
        // strings, ifs, etc. nested inside this def body.
        ruby_prism::visit_def_node(self, node);
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        if let Some(superclass) = node.superclass() {
            let loc = node.location();
            let super_loc = superclass.location();
            self.group_blocking_ranges
                .push((loc.start_offset(), super_loc.end_offset()));
        }
        ruby_prism::visit_class_node(self, node);
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

            // Prism models `%`-newline delimiters like:
            //
            //   x = %
            //   "#{foo}"
            //
            // as a multiline InterpolatedStringNode whose opening token ends
            // with `\n`. RuboCop still treats these as safe-to-split because
            // the delimiter newline is not part of the effective string value.
            if open.as_slice().ends_with(b"\n") {
                ruby_prism::visit_interpolated_string_node(self, node);
                return;
            }
        }
        // Multiline interpolated strings are unsafe in RuboCop even when the
        // embedded send would fit on one line by itself. Treat only LITERAL
        // newlines as unsafe here. Backslash string continuations such as
        // `"foo #{x}" \` newline `"bar #{y}"` should still be checkable.
        if contains_non_continuation_newline(node.location().as_slice()) {
            let loc = node.location();
            self.ranges.push((loc.start_offset(), loc.end_offset()));
        }
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
            self.group_blocking_ranges
                .push((loc.start_offset(), loc.end_offset()));
        }
        // Still recurse into children to find nested unsafe constructs
        ruby_prism::visit_parentheses_node(self, node);
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        self.group_blocking_ranges
            .push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_until_node(self, node);
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        self.group_blocking_ranges
            .push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_while_node(self, node);
    }

    fn visit_for_node(&mut self, node: &ruby_prism::ForNode<'pr>) {
        let loc = node.location();
        self.ranges.push((loc.start_offset(), loc.end_offset()));
        self.group_blocking_ranges
            .push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_for_node(self, node);
    }

    fn visit_regular_expression_node(&mut self, node: &ruby_prism::RegularExpressionNode<'pr>) {
        if node.unescaped().contains(&b'\n') {
            let loc = node.location();
            self.ranges.push((loc.start_offset(), loc.end_offset()));
        }
        ruby_prism::visit_regular_expression_node(self, node);
    }

    fn visit_interpolated_regular_expression_node(
        &mut self,
        node: &ruby_prism::InterpolatedRegularExpressionNode<'pr>,
    ) {
        if node.location().as_slice().contains(&b'\n') {
            let loc = node.location();
            self.ranges.push((loc.start_offset(), loc.end_offset()));
        }
        ruby_prism::visit_interpolated_regular_expression_node(self, node);
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

/// Collects byte ranges of single-line block/lambda nodes whose parent (in Parser AST
/// terms) is a send with a dot.
///
/// Matches RuboCop's `other_cop_takes_precedence?` which checks:
///   `block_node.parent.send_type? && block_node.parent.loc.dot && !block_node.multiline?`
///
/// In Parser AST, `block_node.parent` is the CONTAINING node. For:
///   - `foo.map { ... }.compact` → block parent is `.compact` send (has dot) ✓
///   - `assoc.has_many :x, -> { ... }, ...` → lambda block parent is `.has_many` send ✓
///   - `foo.bar(proc { ... })` → block parent is `.bar` send (has dot) ✓
///   - `x = foo.map { ... }` → block parent is assignment (no dot) ✗
///   - `bar(proc { ... })` → block parent is `bar` send (no dot) ✗
///
/// In Prism, a block is always a child of its "owning" CallNode. The "containing"
/// node in Parser AST terms is found by skipping the owning CallNode and any
/// ArgumentsNode wrapper in the ancestor stack. A `LambdaNode` is already the
/// block wrapper, so only Prism-only argument wrappers are skipped.
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
        if start_line == end_line && self.containing_call_has_dot_for_block() {
            self.ranges.push((loc.start_offset(), loc.end_offset()));
        }
        ruby_prism::visit_block_node(self, node);
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        let loc = node.location();
        let (start_line, _) = self.source.offset_to_line_col(loc.start_offset());
        let (end_line, _) = self
            .source
            .offset_to_line_col(loc.end_offset().saturating_sub(1));
        if start_line == end_line && self.containing_call_has_dot_for_lambda() {
            self.ranges.push((loc.start_offset(), loc.end_offset()));
        }
        ruby_prism::visit_lambda_node(self, node);
    }
}

impl SingleLineBlockCollector<'_, '_> {
    fn call_has_dot(call: &ruby_prism::CallNode<'_>) -> bool {
        call.call_operator_loc()
            .is_some_and(|loc| loc.as_slice() == b".")
    }

    /// Check if the block's "parent" in Parser AST terms is a CallNode with a dot.
    ///
    /// Walk up ancestors, skipping the owning CallNode (immediate parent of the
    /// block in Prism) and any ArgumentsNode wrapper (Prism-only, no Parser
    /// equivalent). The next significant node is the "containing" context.
    fn containing_call_has_dot_for_block(&self) -> bool {
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
            if !skipped_owning_call && ancestor.as_call_node().is_some() {
                skipped_owning_call = true;
                continue;
            }
            // ArgumentsNode is a Prism wrapper with no Parser AST equivalent.
            if ancestor.as_arguments_node().is_some() {
                continue;
            }
            // Found the containing node. Check if it's a CallNode with a dot.
            return ancestor
                .as_call_node()
                .is_some_and(|call| Self::call_has_dot(&call));
        }
        false
    }

    /// Lambda nodes (`-> { ... }`) are already the block wrapper in Prism, so
    /// there is no "owning" CallNode to skip. Only skip Prism-only arguments
    /// wrappers, then check the containing node directly.
    fn containing_call_has_dot_for_lambda(&self) -> bool {
        let len = self.ancestors.len();
        if len < 1 {
            return false;
        }
        for i in (0..len - 1).rev() {
            let ancestor = &self.ancestors[i];
            if ancestor.as_arguments_node().is_some() {
                continue;
            }
            return ancestor
                .as_call_node()
                .is_some_and(|call| Self::call_has_dot(&call));
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
    single_line_block_chain_enabled: bool,
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
    ///
    /// Matches RuboCop's `to_single_line` method which merges string
    /// continuations across backslash: `"foo" \ "bar"` → `"foobar"` (same
    /// quotes merged), `"foo" \ 'bar'` → `"foo" + 'bar'` (different quotes).
    fn too_long(&self, start_offset: usize, end_offset: usize) -> bool {
        let (start_line, _) = self.source.offset_to_line_col(start_offset);
        let (end_line, _) = self
            .source
            .offset_to_line_col(end_offset.saturating_sub(1).max(start_offset));

        let lines: Vec<&[u8]> = self.source.lines().collect();
        let mut combined = Vec::new();
        let mut prev_had_backslash = false;
        for line_num in start_line..=end_line {
            if line_num > lines.len() {
                break;
            }
            let line = lines[line_num - 1];
            // Strip trailing whitespace, then strip trailing backslash (line continuation).
            // Only remove the line-continuation backslash, NOT backslashes that are part of
            // content (e.g., \1_\2 in regex replacements, \d in character classes).
            let trimmed_end = trim_trailing_whitespace(line);
            let had_backslash = trimmed_end.ends_with(b"\\");
            let without_continuation = if had_backslash {
                trim_trailing_whitespace(&trimmed_end[..trimmed_end.len() - 1])
            } else {
                trimmed_end
            };
            if combined.is_empty() {
                combined.extend_from_slice(without_continuation);
            } else {
                let trimmed = trim_leading_whitespace(without_continuation);
                // RuboCop's to_single_line merges string literals across backslash:
                //   /(["']) *\\\n\s*\1/ → '' (same quote = merge)
                //   /" *\\\n\s*'/ → '" + \'' (different quotes)
                if prev_had_backslash && merge_string_continuation(&mut combined, trimmed) {
                    // Merged string continuation — already handled
                } else if starts_with_method_chain_dot(trimmed)
                    || (ends_with_safe_navigation_operator(&combined)
                        && trimmed.first().is_some_and(|b| is_word_char(*b)))
                {
                    combined.extend_from_slice(trimmed);
                } else {
                    combined.push(b' ');
                    combined.extend_from_slice(trimmed);
                }
            }
            prev_had_backslash = had_backslash;
        }

        utf8_char_count(&combined) > self.max_line_length
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
        if self.single_line_block_chain_enabled
            && self.contains_single_line_block(start_offset, end_offset)
        {
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

    /// Returns true if the current call is inside a binary operator node
    /// (`||`/`&&`/`or`/`and`) AND the operator's line does NOT end with `\`.
    ///
    /// In RuboCop, `on_send` walks up from the inner send through parent sends,
    /// convertible blocks, AND `BinaryOperatorNode` parents (OrNode/AndNode).
    /// The walked-up node then undergoes an `operator_keyword?` check: if true,
    /// the offense is gated on `require_backslash?` (the operator line must end
    /// with `\`). Since `operator_keyword?` returns true for both `||`/`or` and
    /// `&&`/`and`, a multiline call inside `foo || bar(...)` is NOT flagged
    /// unless the `||` line ends with `\`.
    ///
    /// Nitrocop's AST visitor doesn't walk up through OrNode/AndNode, so it
    /// would incorrectly flag the inner call. This method detects the situation
    /// and suppresses the offense.
    fn inside_binary_op_without_backslash(&self) -> bool {
        // Walk up ancestors following RuboCop's on_send walk-up rules:
        //   - CallNode (parent send) → continue
        //   - ArgumentsNode (Prism wrapper) → continue
        //   - BlockNode (convertible block) → continue
        //   - OrNode/AndNode → found! check backslash
        //   - Anything else → stop
        if self.ancestors.len() < 2 {
            return false;
        }
        for i in (0..self.ancestors.len() - 1).rev() {
            let ancestor = &self.ancestors[i];

            if ancestor.as_call_node().is_some() || ancestor.as_arguments_node().is_some() {
                continue;
            }

            // Convertible blocks: in RuboCop, the walk-up goes through blocks
            // whose send is parenthesized or has no args. Be a bit generous and
            // continue through any BlockNode.
            if ancestor.as_block_node().is_some() {
                continue;
            }

            // Found an OrNode or AndNode — check if its operator line ends with `\`.
            let operator_loc = ancestor
                .as_or_node()
                .map(|n| n.operator_loc())
                .or_else(|| ancestor.as_and_node().map(|n| n.operator_loc()));

            if let Some(op_loc) = operator_loc {
                let (op_line, _) = self.source.offset_to_line_col(op_loc.start_offset());
                let lines: Vec<&[u8]> = self.source.lines().collect();
                if op_line > 0 && op_line <= lines.len() {
                    let line = lines[op_line - 1];
                    let trimmed = trim_trailing_whitespace(line);
                    return !trimmed.ends_with(b"\\");
                }
                return true;
            }

            // Any other node type stops the walk-up.
            break;
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

    fn receiver_chain_contains_safe_navigation(&self, node: &ruby_prism::CallNode<'pr>) -> bool {
        node.call_operator_loc()
            .is_some_and(|loc| loc.as_slice() == b"&.")
            || node
                .receiver()
                .and_then(|receiver| receiver.as_call_node())
                .is_some_and(|receiver| self.receiver_chain_contains_safe_navigation(&receiver))
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

        let skip_for_checked_chain =
            !has_non_convertible_block && self.part_of_checked_chain(start_offset, end_offset);
        let unary_bang_wrapper = node.name().as_slice() == b"!"
            && node.arguments().is_none()
            && node.block().is_none()
            && node.receiver().and_then(|r| r.as_call_node()).is_some();
        let safe_navigation_unary_wrapper = unary_bang_wrapper
            && node
                .receiver()
                .and_then(|receiver| receiver.as_call_node())
                .is_some_and(|receiver| self.receiver_chain_contains_safe_navigation(&receiver));

        if safe_navigation_unary_wrapper {
            ruby_prism::visit_call_node(self, node);
            return;
        }

        if self.is_multiline(start_offset, check_end)
            && !self.part_of_reported_node(start_offset, end_offset)
            && !skip_for_checked_chain
        {
            // RuboCop's `on_send` walks up through parent sends even when the
            // inner send is a direct argument of the outer one, not only when
            // it is the receiver in a method chain. Record every multiline send
            // range so nested direct-argument sends are skipped unless another
            // structural boundary (hash/array/parentheses/block body) breaks
            // the walk-up.
            //
            // Exclude block bodies: in RuboCop, the send node's range does not
            // include the block (the block is a parent node). Calls inside
            // block bodies should be checked independently.
            let effective_end = node
                .block()
                .and_then(|b| b.as_block_node())
                .map_or(end_offset, |block| block.location().start_offset());
            self.checked_chain_ranges
                .push((start_offset, effective_end));

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
                && !self.inside_binary_op_without_backslash()
                && self.suitable_as_single_line(start_offset, check_end)
                && !self.configured_to_not_be_inspected(start_offset, check_end)
                && (!has_convertible_block || self.inspect_blocks)
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
    group_blocking_ranges: &[(usize, usize)],
    checked_chain_ranges: &[(usize, usize)],
    block_ranges: &[(usize, usize, bool)],
    comment_lines: &HashSet<usize>,
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

        // RuboCop never reports `class Foo < \` superclass header breaks here.
        if trimmed_content.starts_with(b"class ") && trimmed_content.contains(&b'<') {
            i += 1;
            continue;
        }

        // A backslash on an already-dotted chain segment (e.g. `.replace(...) \`)
        // belongs to the larger multiline chain, which RuboCop judges as a whole.
        if starts_with_method_chain_dot(trimmed_content) {
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

        // Phase 2 starts from explicit backslash continuations, but the Ruby
        // expression can keep going across later comma-terminated lines:
        //
        //   attr_reader \
        //     :foo,
        //     :bar,
        //     :baz
        //
        // RuboCop measures the full continued call here, not just `attr_reader :foo,`.
        let mut expression_end_idx = final_line_idx;
        while expression_end_idx + 1 < lines.len() {
            let current = trim_trailing_whitespace(lines[expression_end_idx]);
            if current.is_empty() || !current.ends_with(b",") {
                break;
            }

            let next_trimmed_content =
                trim_leading_whitespace(trim_trailing_whitespace(lines[expression_end_idx + 1]));
            if next_trimmed_content.starts_with(b"#") {
                break;
            }

            expression_end_idx += 1;
        }

        let report_line = group_start + 1; // 1-indexed
        if already_reported.contains(&report_line) {
            i = expression_end_idx + 1;
            continue;
        }

        // Check if the backslash group's byte range overlaps with any unsafe
        // construct (if/case/begin/def/heredoc/multiline-string) or block.
        // This matches RuboCop's AST-level checks that prevent collapsing
        // expressions containing these constructs.
        let group_byte_start = line_starts[group_start];
        let group_byte_end = if expression_end_idx < line_starts.len() {
            line_starts[expression_end_idx] + lines[expression_end_idx].len()
        } else {
            content.len()
        };

        // Check if any unsafe range is fully contained within the group OR
        // starts within the group but extends beyond it. The latter catches
        // backslash continuations followed by case/if(ternary)/until/while
        // expressions: the construct starts in the continuation line but
        // extends far past the group, so it can't be collapsed to one line.
        // We intentionally don't check for unsafe ranges that merely CONTAIN
        // the group (like def bodies) — those are legitimate contexts for
        // backslash continuation offenses.
        let has_unsafe = unsafe_ranges
            .iter()
            .any(|&(us, _ue)| us >= group_byte_start && us < group_byte_end);
        if has_unsafe {
            i = final_line_idx + 1;
            continue;
        }

        // Some constructs should suppress the text fallback even when the
        // unsafe node starts before the backslash group. Examples:
        // - `class Foo < \` newline `Bar`
        // - backslash groups nested inside a larger multiline `CallNode`
        //   that the AST phase already judged as a whole.
        let blocked_by_enclosing_range = group_blocking_ranges
            .iter()
            .any(|&(bs, be)| bs <= group_byte_start && be >= group_byte_end);
        if blocked_by_enclosing_range {
            i = expression_end_idx + 1;
            continue;
        }

        let covered_by_checked_chain = checked_chain_ranges.iter().any(|&(cs, ce)| {
            cs <= group_byte_start
                && ce >= group_byte_end
                && (cs < group_byte_start || ce > group_byte_end)
        });
        if covered_by_checked_chain {
            i = expression_end_idx + 1;
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

        let has_comment = ((group_start + 1)..=(expression_end_idx + 1))
            .any(|line_num| comment_lines.contains(&line_num));
        if has_comment {
            i = expression_end_idx + 1;
            continue;
        }

        // Build the combined single-line version.
        let indent = leading_whitespace_len(lines[group_start]);
        let mut combined = Vec::new();
        combined.extend_from_slice(&lines[group_start][..indent]);

        for (j, line_idx) in (group_start..=expression_end_idx).enumerate() {
            let t = trim_trailing_whitespace(lines[line_idx]);
            if t.is_empty() {
                continue;
            }
            let content_part = if line_idx <= group_end {
                let before_bs = trim_trailing_whitespace(&t[..t.len() - 1]);
                trim_leading_whitespace(before_bs)
            } else {
                trim_leading_whitespace(t)
            };

            if j == 0 {
                combined.extend_from_slice(content_part);
            } else {
                combined.push(b' ');
                combined.extend_from_slice(content_part);
            }
        }

        if utf8_char_count(&combined) > max_line_length {
            i = expression_end_idx + 1;
            continue;
        }

        let next_content = trim_leading_whitespace(lines[group_start + 1]);
        if next_content.starts_with(b"&&") || next_content.starts_with(b"||") {
            i = expression_end_idx + 1;
            continue;
        }

        if is_string_concat_continuation(&lines, group_start, group_end) {
            i = expression_end_idx + 1;
            continue;
        }

        diagnostics.push(cop.diagnostic(
            source,
            report_line,
            0,
            "Redundant line break detected.".to_string(),
        ));

        i = expression_end_idx + 1;
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
                if j + 1 == group_end && is_branch_terminator(next_content) {
                    break;
                }
                return false;
            }
        }
    }
    if group_end < lines.len() {
        let tail_content = trim_leading_whitespace(trim_trailing_whitespace(lines[group_end]));
        if tail_content.is_empty() {
            return false;
        }
        let first_char = tail_content[0];
        if first_char == b'\'' || first_char == b'"' {
            return true;
        }
        if is_branch_terminator(tail_content) {
            return true;
        }
        return false;
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

/// Merge string continuation across a backslash line break, matching
/// RuboCop's `to_single_line` regex patterns:
///   - `/(["']) *\\\n\s*\1/` → `''` (same quote: merge the strings)
///   - `/" *\\\n\s*'/` → `" + '` (different quotes: use concatenation)
///   - `/' *\\\n\s*"/` → `' + "` (different quotes: use concatenation)
///
/// Returns true if a merge was performed, false otherwise.
fn merge_string_continuation(combined: &mut Vec<u8>, next_trimmed: &[u8]) -> bool {
    if combined.is_empty() || next_trimmed.is_empty() {
        return false;
    }
    let last = combined[combined.len() - 1];
    let first = next_trimmed[0];
    if last != b'"' && last != b'\'' {
        return false;
    }
    if first != b'"' && first != b'\'' {
        return false;
    }
    if last == first {
        // Same quote: merge the two string literals into one
        // "foo" \ "bar" → "foobar"
        combined.pop(); // Remove trailing quote
        combined.extend_from_slice(&next_trimmed[1..]); // Skip leading quote
    } else {
        // Different quotes: use + operator
        // "foo" \ 'bar' → "foo" + 'bar'
        combined.extend_from_slice(b" + ");
        combined.extend_from_slice(next_trimmed);
    }
    true
}

fn ends_with_safe_navigation_operator(trimmed: &[u8]) -> bool {
    trimmed.ends_with(b"&.")
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

fn contains_non_continuation_newline(bytes: &[u8]) -> bool {
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'\n' {
            continue;
        }

        let mut j = i;
        while j > 0 && (bytes[j - 1] == b' ' || bytes[j - 1] == b'\t') {
            j -= 1;
        }

        if j == 0 || bytes[j - 1] != b'\\' {
            return true;
        }
    }
    false
}

fn is_branch_terminator(trimmed: &[u8]) -> bool {
    trimmed == b"end"
        || trimmed == b"else"
        || trimmed == b"ensure"
        || trimmed.starts_with(b"elsif ")
        || trimmed.starts_with(b"when ")
        || trimmed.starts_with(b"rescue ")
}

/// Count the number of Unicode characters (code points) in a UTF-8 byte slice.
/// RuboCop measures line length in characters, not bytes. For multi-byte UTF-8
/// (e.g. CJK characters), byte length > char length, causing FNs when using
/// byte length.
fn utf8_char_count(bytes: &[u8]) -> usize {
    // UTF-8 continuation bytes match the pattern 10xxxxxx (0x80..0xBF).
    // Every other byte is the start of a new character.
    bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    crate::cop_fixture_tests!(RedundantLineBreak, "cops/layout/redundant_line_break");

    #[test]
    fn safe_navigation_chain_with_trailing_operator_uses_exact_joined_length() {
        let source = b"!current_course_user&.\n  email_unsubscriptions&.\n  where(course_settings_email_id: email_setting_enabled(component, setting).id)&.exists?\n";
        let config = CopConfig {
            options: HashMap::from([(
                "MaxLineLength".to_string(),
                serde_yml::Value::Number(serde_yml::Number::from(132)),
            )]),
            ..CopConfig::default()
        };

        let diagnostics =
            crate::testutil::run_cop_full_with_config(&RedundantLineBreak, source, config);

        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].location.line, 1);
        assert_eq!(diagnostics[0].location.column, 1);
    }

    #[test]
    fn reports_single_line_block_chains_when_single_line_block_chain_is_disabled() {
        let source = b"e.select { |i| i.cond? }\n  .join\n";
        let config = CopConfig {
            options: HashMap::from([(
                "SingleLineBlockChainEnabled".to_string(),
                serde_yml::Value::Bool(false),
            )]),
            ..CopConfig::default()
        };

        let diagnostics =
            crate::testutil::run_cop_full_with_config(&RedundantLineBreak, source, config);

        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].location.line, 1);
        assert_eq!(diagnostics[0].location.column, 0);
    }

    #[test]
    fn skips_single_line_block_chains_when_single_line_block_chain_is_enabled() {
        let source = b"e.select { |i| i.cond? }\n  .join\n";
        let config = CopConfig {
            options: HashMap::from([(
                "SingleLineBlockChainEnabled".to_string(),
                serde_yml::Value::Bool(true),
            )]),
            ..CopConfig::default()
        };

        let diagnostics =
            crate::testutil::run_cop_full_with_config(&RedundantLineBreak, source, config);

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn stabby_lambda_arguments_respect_single_line_block_chain_config() {
        let source = b"foo.bar :x,\n  -> { baz }\n";

        let disabled = CopConfig {
            options: HashMap::from([(
                "SingleLineBlockChainEnabled".to_string(),
                serde_yml::Value::Bool(false),
            )]),
            ..CopConfig::default()
        };
        let diagnostics =
            crate::testutil::run_cop_full_with_config(&RedundantLineBreak, source, disabled);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].location.line, 1);
        assert_eq!(diagnostics[0].location.column, 0);

        let enabled = CopConfig {
            options: HashMap::from([(
                "SingleLineBlockChainEnabled".to_string(),
                serde_yml::Value::Bool(true),
            )]),
            ..CopConfig::default()
        };
        let diagnostics =
            crate::testutil::run_cop_full_with_config(&RedundantLineBreak, source, enabled);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}
