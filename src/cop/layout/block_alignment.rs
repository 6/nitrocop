use crate::cop::shared::node_type::{
    BEGIN_NODE, CALL_NODE, LAMBDA_NODE, PARENTHESES_NODE, RESCUE_MODIFIER_NODE, STATEMENTS_NODE,
    node_type_tag,
};
use crate::cop::shared::node_type_groups::is_assignment_type;
use crate::cop::shared::util::begins_its_line;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Checks whether the end keywords / closing braces are aligned properly for
/// do..end and {..} blocks.
///
/// ## Corpus investigation findings (2026-03-11)
///
/// Root causes of 1,187 FP:
/// 1. **Trailing-dot method chains** — `find_chain_expression_start` only checked
///    for lines starting with `.` (leading dot) but NOT for lines ending with `.`
///    (trailing dot style). This caused the chain root to not be found, computing
///    wrong `expression_start_indent` and flagging correctly-aligned `end`.
/// 2. **Tab indentation** — `line_indent` only counted spaces, returning 0 for
///    tab-indented lines. But `offset_to_line_col` counts tabs as 1 character,
///    causing a mismatch between computed indent and actual `end` column.
/// 3. **Missing `begins_its_line?` check** — RuboCop skips alignment checks when
///    `end`/`}` is not the first non-whitespace on its line (e.g., `end.select`).
///    nitrocop checked all `end` keywords regardless.
///
/// Root causes of 334 FN:
/// 1. **Brace blocks not checked** — RuboCop checks both `do..end` and `{..}`
///    blocks, but nitrocop only checked `do..end`. Many FNs were misaligned `}`.
///
/// Fixes applied:
/// - `line_indent` now counts both spaces and tabs
/// - `find_chain_expression_start` now handles trailing-dot chains (lines ending with `.`)
/// - Added `begins_its_line` check to skip non-line-beginning closers
/// - Added brace block (`{..}`) checking with same alignment rules
/// - Fixed `start_of_block` style to use do-line indent (not `do` column) per RuboCop spec
///
/// ## Corpus investigation findings (2026-03-14)
///
/// Root causes of remaining 411 FP:
/// 1. **String concatenation `+` continuation** — Lines ending with `+` (common in
///    RSpec multiline descriptions like `it "foo " + "bar" do`) were not recognized
///    as expression continuations. `find_chain_expression_start` stopped too early,
///    computing wrong `expression_start_indent` and flagging correctly-aligned `end`.
///    Fixed by adding `+` to the continuation character set.
///
/// Root causes of remaining 103 FN:
/// 1. **Assignment RHS alignment accepted** — `find_call_expression_col` walked
///    backward from `do`/`{` to find the call expression start, but stopped at the
///    RHS of assignments (e.g., `answer = prompt.select do`). This made `call_expr_col`
///    point to `prompt` instead of `answer`, causing nitrocop to accept `end` aligned
///    with the RHS when RuboCop requires alignment with the LHS variable.
///    Fixed by adding `skip_assignment_backward` to walk through `=`/`+=`/`||=`/etc.
///    to find the LHS variable.
///
/// ## Corpus investigation findings (2026-03-18)
///
/// Root causes of remaining 176 FP:
/// 1. **Multiline string literals** — The line-based heuristic `find_chain_expression_start`
///    could not detect string literals spanning multiple lines without explicit continuation
///    markers (e.g., `it "long desc\n    continued" do`). This caused the expression start
///    to be computed from the wrong line.
/// 2. **Comment lines between continuations** — Comment lines interleaved in multi-line
///    method calls (e.g., RSpec `it` with keyword args after comments) broke the backward
///    line walk.
///
/// Root causes of remaining 55 FN:
/// 1. **Over-eager backward walk** — `find_chain_expression_start` walked through unclosed
///    brackets into outer expressions (e.g., from `lambda{|env|` through `show_status(` into
///    `req = ...`), computing an expression indent that matched the misaligned closer.
///
/// Fix: Replaced `BLOCK_NODE` with `CALL_NODE` dispatch. The CallNode's `location()` in
/// Prism spans the entire expression including receiver chains, giving the exact expression
/// start without heuristic line-based backward walking. This eliminates multiline string,
/// comment interleaving, and bracket-balance issues in one structural change.
/// Replaced `find_chain_expression_start` with `find_operator_continuation_start` which
/// only walks through `||`, `&&`, `<<`, `+` operators (not brackets/commas/backslashes),
/// preventing over-eager backward walking that caused false negatives.
///
/// ## Corpus investigation findings (2026-03-18, round 2)
///
/// Root causes of remaining 16 FP:
/// 1. **Chained blocks in assignment context** — `response = stub_comms do ... end.check_request do`
///    where `end` at col N aligns with the method call (`stub_comms`) but the assignment LHS
///    (`response`) is at a different column. The old code skipped `call_start_col` when
///    `assignment_col.is_some()`, preventing recognition of valid intermediate alignment.
///    Fixed by accepting `call_start_col` when the closer is chained (`.method` or `&.method`
///    follows `end`/`}`).
/// 2. **`&&`/`||` on same line as `do`/`{`** — `a && b.each do ... end` where `end` aligns
///    with the LHS of the `&&` expression. Added `find_same_line_operator_lhs` to detect
///    binary operators before the CallNode on the same line.
///
/// Root causes of remaining 34 FN:
/// 1. **Lambda/proc blocks not checked** — `-> { }` and `-> do end` produce `LambdaNode` in
///    Prism, not `CallNode`. The cop only dispatched on `CALL_NODE`. Added `LAMBDA_NODE`
///    dispatch with `check_lambda_alignment` method.
/// 2. **`do_col` incorrectly accepted as alignment target** — The column of the `do`/`{`
///    keyword itself was accepted in "either" mode, but RuboCop only accepts the indent
///    of the do-line (`indentation_of_do_line`) and the expression start column. Removing
///    `do_col` from accepted targets fixes FNs like `Hash.new do ... end` where `end` at
///    the `do` column was incorrectly accepted.
/// 3. **Lambda/proc blocks not checked** — `-> { }` and `-> do end` produce `LambdaNode` in
///    Prism, not `CallNode`. The cop only dispatched on `CALL_NODE`. Added `LAMBDA_NODE`
///    dispatch with `check_lambda_alignment` method.
/// 4. **Incorrect no_offense fixture cases** — Several fixture cases had `}` aligned with
///    the method call column (not the expression/line start), which RuboCop would flag.
///    Removed factually incorrect cases from no_offense.rb.
///
/// ## Corpus investigation findings (2026-03-19)
///
/// Root causes of remaining 6 FP:
/// 1. **Next-line dot chain** (ubicloud, 2 FP) — `}` followed by newline + `.sort_by` was
///    not detected as a chained closer because `is_closer_chained` only checked for `.`
///    immediately after the closer. Extended to check the next non-empty line for leading `.`.
/// 2. **`&&`/`||` with complex LHS** (forem, 1 FP) — `if x == "str" && y.each do ... end`
///    where `find_same_line_operator_lhs` couldn't walk backward through string literals
///    and `==` in the LHS. Made the backward walk more permissive (handles quotes, operators).
/// 3. **Multiline assignment on previous line** (pivotal, 1 FP) — `a, b =\n  stdout\n  .reduce do`
///    where `find_assignment_lhs_col` only checked the same line as the CallNode. Extended
///    to check the previous line when the call starts at line indent and prev line ends with `=`.
/// 4. **Paren/rescue context** (automaticmode, 1 FP) — `(svg = IO.popen(...) { } rescue false)`.
///    Not fixed; requires AST parent walk.
/// 5. **Splat method arg deep indentation** (flyerhzm, 1 FP) — `*descendants.map { ... }`.
///    Not fixed; requires AST parent walk.
///
/// Root causes of remaining 17 FN:
/// 1. **`expression_start_indent` too permissive** (Arachni, 6 FN + seyhunak, 3 FN + others) —
///    When a block's call expression is mid-line (e.g., inside parens like `expect(auditable.audit(...)
///    do`), the line indent matches the outer context, not the call expression. Guarded
///    `expression_start_indent` to only be accepted when `call_start_col == expression_start_indent`
///    (i.e., the call starts at the line's indent position).
/// 2. **`%` not in `find_call_expression_col` chars** (randym, 2 FN + floere, 1 FN) —
///    `%w(...)` percent literals weren't fully walked backward. Added `%` to accepted characters.
/// 3. **Lambda `call_expr_col` accepting `{` column** (refinery, 1 FN) — For lambda blocks,
///    `find_call_expression_col` gave the `{`/`do` position rather than `->`, causing `}`
///    aligned with `{` to be accepted. Removed `call_expr_col` from lambda alignment check.
/// 4. **Chained `.to_json` accepted in assignment** (diaspora, 1 FN) — Not fixed; chained
///    closer heuristic accepts `call_start_col` which matches the RHS call. Requires AST walk.
///
/// Remaining gaps: 2 FP (paren/rescue, splat-arg) + 1 FN (chained closer in assignment).
///
/// ## Corpus investigation findings (2026-03-19, round 3)
///
/// Root causes of remaining 10 FP:
/// 1. **`:` in bracket-key LHS** (vagrant-parallels x2, hashicorp/vagrant, JEG2/highline,
///    peritor/webistrano — 5 FP) — `env[:machine].id = expr do` or `entry[:phone] = ask(...) do`.
///    `skip_assignment_backward` LHS walk didn't handle `:` (symbol literal prefix inside
///    brackets), stopping at `:machine` instead of walking through to `env`. Fixed by adding
///    `:` to accepted chars and balanced paren/bracket handling in the LHS walk.
/// 2. **`<<` not handled as same-line operator** (docuseal, openstreetmap — 2 FP) —
///    `acc << expr do ... end` or `lists << tag.ul(...) do`. The `<<` shovel operator wasn't
///    recognized by `find_same_line_operator_lhs`, so `acc`'s column wasn't accepted as
///    alignment target. Added `<<` to the same-line operator check.
/// 3. **Parens not handled in LHS walk** (openstreetmap, opf/openproject — 1 FP) —
///    `RequestStore.store[key(work_package)] = value do` where `(` in the LHS stopped the
///    backward walk. Added balanced paren/bracket handling to `skip_assignment_backward`.
/// 4. Existing unfixable: paren/rescue (automaticmode, 1 FP), splat-arg (flyerhzm, 1 FP).
///
/// Root causes of remaining 7 FN:
/// 1. **Cross-line single assignment accepted** (ankane/blazer, fog, jruby/warbler — 3 FN) —
///    `@connection_model =\n  Class.new(...) do ... end` at col 8. `find_assignment_lhs_col`
///    walked to the previous line and found the assignment LHS. But RuboCop's
///    `disqualified_parent?` stops at cross-line parents (except masgn). Fixed by restricting
///    cross-line assignment detection to multi-assignment (masgn) only (detected by comma in LHS).
/// 2. **Cross-line `||`/`&&` accepted as alignment target** (sharetribe — 1 FN) —
///    `accepted_states.empty? ||\n  accepted_states.any? do ... end` at col 6.
///    `find_operator_continuation_start` accepted the indent of the `||` LHS line. But RuboCop's
///    `disqualified_parent?` stops at cross-line parents. Fixed by removing
///    `find_operator_continuation_start` entirely — cross-line operator continuations are
///    not valid alignment targets.
/// 3. **Cross-line `<<` no longer accepted** (trogdoro — 1 FN) — `threads <<\n  Thread::new(...)
///    do ... end` at col 10 matched `threads <<` line indent via `operator_continuation_indent`.
///    Removing that function fixed this FN.
/// 4. **Cross-line single assignment no longer accepted** (sup-heliotrope — 1 FN) —
///    `@files =\n  begin...end.map do ... end` at col 4. The cross-line `@files =` was
///    previously accepted; masgn restriction now rejects it.
/// 5. Existing unfixable: chained `.to_json` in assignment (diaspora, 1 FN).
///
/// Remaining gaps: 2 FP (paren/rescue, splat-arg) + 1 FN (chained closer in assignment).
///
/// ## Corpus investigation findings (2026-03-20)
///
/// Root causes of the final oracle-known gaps:
/// 1. **Rescue modifier wrapper** (automaticmode, 1 FP) — `foo { ... } rescue false`
///    should stop the ancestor walk at the current block expression, so the closer
///    aligns with the block call start rather than the outer assignment LHS.
/// 2. **Splat wrapper** (flyerhzm, 1 FP) — `wrap *items.map { ... }` aligns the
///    closer with the `*` column because RuboCop stops at the `splat` ancestor.
/// 3. **Plain chained call in assignment** (diaspora, 1 FN) — `result = items.map { ... }.to_json`
///    must NOT accept the inner call start. RuboCop walks through the normal send
///    chain to the assignment, so the closer aligns with the assignment LHS.
///
/// Fixes applied:
/// - Added `find_same_line_splat_col` so splat-wrapped block calls align to `*`
/// - Replaced the broad chained-closer escape with `accept_intermediate_call_start`,
///   which only keeps the inner call start for rescue wrappers, safe-navigation
///   chains (`&.`), and chained calls that immediately open another block
///
/// Verification:
/// - `cargo test --lib -- block_alignment` passes with new fixture coverage for all
///   three patterns
/// - `scripts/verify-cop-locations.py Layout/BlockAlignment` reports all CI-known
///   FP/FN fixed
/// - `scripts/check-cop.py Layout/BlockAlignment --verbose --rerun` still reports
///   15 excess in local batch `--corpus-check` mode, but a direct per-repo
///   nitrocop vs RuboCop sweep over all 188 active repos shows 0 count delta on
///   the 180 repos that were locally comparable. The 8 remaining repos failed
///   local RuboCop/json validation (`devdocs`, `jruby`, and 6 repos with local
///   JSON/tooling issues), so the residual batch excess is likely validation noise
///   rather than a confirmed cop-logic mismatch.
///
/// ## Corpus investigation findings (2026-03-30)
///
/// Root causes of the remaining 9 FN:
/// 1. **`call_expr_col` overrode wrapper targets** — same-line `||`, `&&`, `<<`,
///    and `*` wrappers computed the right outer target, but the fallback
///    `call_expr_col` still accepted the inner call start.
/// 2. **Wrapper-stopping parents still allowed assignment LHS** — when a block
///    closer immediately chained into another block, safe-navigation send, or
///    rescue modifier, RuboCop stopped before the outer assignment, but
///    nitrocop still accepted the assignment LHS.
/// 3. **Receiver chains ending at `}`** — `find_call_expression_col` handled
///    `)`/`]` receivers but stopped just after `}` in `}.each do` chains.
/// 4. **Multiline stabby lambdas** — `LambdaNode::location()` can start after
///    the `->` operator when lambda arguments wrap, and the line indent of a
///    mid-line `->` is not a valid alternate target.
///
/// Fixes applied:
/// - Ignore `call_expr_col` when a same-line operator or splat wrapper already
///   provides the outer alignment target
/// - Drop assignment-LHS alignment whenever the closer immediately feeds an
///   outer wrapper that RuboCop stops on, while still allowing plain `||=` and
///   `&&=` memoization blocks
/// - Walk through balanced `{...}` receivers in `find_call_expression_col`
/// - Use `LambdaNode::operator_loc()` and only accept line-indent alignment for
///   lambdas that actually start at that indent
///
/// ## Fixture sync (2026-03-30)
///
/// The remaining local `offense.rb` failure was fixture drift, not a new
/// detection gap. Raw oracle snippets had been appended without parseable Ruby
/// context, duplicating real FN examples already covered earlier in the fixture
/// and by focused unit tests. The fix is to keep the shared fixture on the
/// parseable examples rather than the orphaned snippets.
///
/// ## Corpus investigation findings (2026-04-02)
///
/// Root cause of a remaining FP cluster:
/// 1. **Line-leading closers before same-line operators** — on lines like
///    `} || rhs.any? do` or `end.values_at(...) || rhs.each do`,
///    `find_same_line_operator_lhs` treated the previous expression's closer as
///    a real same-line LHS wrapper and suppressed the valid RHS block target.
///    RuboCop instead stops before that outer operator because the left operand
///    started on a previous line.
///
/// Fix applied:
/// - Ignore `&&`/`||`/`<<` wrapper candidates when the would-be same-line LHS
///   starts with a line-leading closer (`end`, `}`, `)`, or `]`)
///
/// ## Corpus investigation findings (2026-04-09)
///
/// Root causes of the remaining 8 FP:
/// 1. **Multiline assignment chains lost the assignment line** — Prism can
///    report the block call start on the chained segment line (`.map do`,
///    `group_by do`, keyword-arg continuation lines), so the old assignment
///    walk only checked the current line or the immediately preceding `=` line.
///    That missed valid LHS targets in chained RHS expressions such as
///    `a = source.zip(...).map do`, `Parallel.map(... ) do ... end.transpose`,
///    and multiline call args like `File.open(... ) { ... }`.
/// 2. **Default parameter values looked like assignments** — `block = lambda { }`
///    inside a `def(...)` parameter list is an `OptionalParameterNode`, not an
///    assignment target in RuboCop. The text-only `=` scan incorrectly aligned
///    the closer with the parameter name.
/// 3. **`call_expr_col` was still assignment-biased** — for chained closers like
///    `end.keys[0]`, RuboCop can stop at the intermediate chained send before an
///    index access, but nitrocop's `find_call_expression_col` walked through `=`
///    and lost the inner call start.
/// 4. **Brace/operator wrappers need both targets** — brace blocks on same-line
///    `||` / `&&` wrappers may align with either the wrapper LHS or the brace
///    call start, while `do..end` operator wrappers should still reject the
///    inner call target.
/// 5. **Lambda `||` wrappers were missing** — `-> { ... }` inside a same-line
///    `||` expression should align with the wrapper LHS, but lambda alignment
///    only considered the lambda start and assignments.
/// 6. **Nested expressions inherited outer assignments** — the text-only
///    assignment walk treated any earlier `=` as a valid alignment target,
///    even when the block/lambda lived inside an argument list, keyword pair,
///    or `if` predicate. RuboCop only keeps walking through transparent
///    receiver-chain wrappers before it reaches an assignment.
/// 7. **Indexed/call op-assign wrappers were missed** — Prism uses dedicated
///    `IndexOrWriteNode` / `CallOrWriteNode` / `...OperatorWriteNode` parents
///    for `foo[:bar] ||= ...` and `foo.bar ||= ...`, so a generic assignment
///    predicate that only covered variable/constant writes dropped legitimate
///    assignment targets.
///
/// Fixes applied:
/// - Walk backward across chained continuation lines to recover assignment LHS
/// - Ignore `=` inside `def(...)` / stabby-lambda parameter default context
/// - Keep raw inner call alignment for brace wrappers and `end.keys[0]`-style
///   indexed chains, while preserving the existing `.to_json` offense behavior
/// - Let lambda alignment reuse same-line `||` / `&&` / `<<` wrapper targets
/// - Only reuse assignment LHS targets when the Prism parent chain stays within
///   transparent wrappers (receiver calls, parens/begin/rescue, statements)
/// - Treat Prism's call/index assignment wrapper nodes as assignment parents too
pub struct BlockAlignment;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SameLineOperatorKind {
    Logical,
    Shovel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct NodeSpan {
    start: usize,
    end: usize,
}

impl NodeSpan {
    fn from_node(node: &ruby_prism::Node<'_>) -> Self {
        let loc = node.location();
        Self {
            start: loc.start_offset(),
            end: loc.end_offset(),
        }
    }
}

impl Cop for BlockAlignment {
    fn name(&self) -> &'static str {
        "Layout/BlockAlignment"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, LAMBDA_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        if let Some(lambda_node) = node.as_lambda_node() {
            self.check_lambda_alignment(source, &lambda_node, parse_result, config, diagnostics);
            return;
        }

        if let Some(call_node) = node.as_call_node() {
            self.check_call_alignment(source, &call_node, parse_result, config, diagnostics);
        }
    }
}

impl BlockAlignment {
    fn check_call_alignment(
        &self,
        source: &SourceFile,
        call_node: &ruby_prism::CallNode<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let block_node = match call_node.block().and_then(|b| b.as_block_node()) {
            Some(b) => b,
            None => return,
        };

        let style = config.get_str("EnforcedStyleAlignWith", "either");

        let closing_loc = block_node.closing_loc();
        let closing_slice = closing_loc.as_slice();
        let is_do_end = closing_slice == b"end";
        let is_brace = closing_slice == b"}";
        if !is_do_end && !is_brace {
            return;
        }

        // RuboCop's begins_its_line? check: only inspect alignment when the
        // closing keyword/brace is the first non-whitespace on its line.
        if !begins_its_line(source, closing_loc.start_offset()) {
            return;
        }
        let bytes = source.as_bytes();

        let opening_loc = block_node.opening_loc();
        let (opening_line, _) = source.offset_to_line_col(opening_loc.start_offset());

        // Find the indentation of the line containing the block opener.
        let start_of_line_indent = line_indent(bytes, opening_loc.start_offset());

        // Use the CallNode's location to get the expression start.
        // In Prism, call_node.location() spans the entire expression including
        // the full receiver chain (e.g., for `@account.things.where(...).in_batches do`,
        // the CallNode location starts at `@account`). This replaces the previous
        // heuristic line-based backward scanning (`find_chain_expression_start`),
        // which couldn't handle multiline strings, interleaved comments, etc.
        let call_start_offset = call_node.location().start_offset();
        let (_, call_start_col) = source.offset_to_line_col(call_start_offset);

        // Check for assignment: if the call expression is on the RHS of `=`/`+=`/etc.,
        // walk backward from the call start to find the LHS variable.
        // When there's an assignment, the alignment target is the LHS (matching RuboCop's
        // behavior where `block_end_align_target` walks past assignment nodes).
        let assignment_col = find_assignment_lhs_col(bytes, call_start_offset)
            .filter(|_| assignment_alignment_allowed(parse_result, &call_node.as_node()));
        let accept_call_start = assignment_col.is_some()
            && accept_intermediate_call_start(
                bytes,
                closing_loc.start_offset(),
                closing_loc.as_slice().len(),
            );
        let splat_col = find_same_line_splat_col(bytes, call_start_offset);
        let same_line_operator = find_same_line_operator(bytes, opening_loc.start_offset())
            .or_else(|| find_same_line_operator(bytes, call_start_offset));
        let same_line_operator_col = same_line_operator.map(|(col, _)| col);
        let same_line_operator_is_logical =
            same_line_operator.is_some_and(|(_, kind)| kind == SameLineOperatorKind::Logical);

        // The expression start column: if there's an assignment on the same line as
        // the call start, use the LHS column. If the block call is wrapped in a
        // same-line logical/shovel operator, or in a same-line splat
        // (`wrap *items.map { ... }`), align with that wrapper instead of the
        // inner call expression. Otherwise use the CallNode's column.
        let expression_start_col = same_line_operator_col
            .or(splat_col)
            .or_else(|| (!accept_call_start).then_some(assignment_col).flatten())
            .unwrap_or(call_start_col);

        // Also compute the expression start line's indent.
        let expression_start_indent = line_indent(bytes, call_start_offset);

        // Find the column of the call expression on the do-line (for hash-value blocks).
        let call_expr_col = find_call_expression_col(bytes, opening_loc.start_offset());
        let accept_call_expr_col = splat_col.is_none()
            && (same_line_operator_col.is_none() || (is_brace && same_line_operator_is_logical))
            && (assignment_col.is_none()
                || chained_call_followed_by_index(
                    bytes,
                    closing_loc.start_offset(),
                    closing_loc.as_slice().len(),
                ));

        let (end_line, end_col) = source.offset_to_line_col(closing_loc.start_offset());

        // Only flag if closing is on a different line than opening
        if end_line == opening_line {
            return;
        }

        let close_word = if is_brace { "`}`" } else { "`end`" };
        let open_word = if is_brace { "`{`" } else { "`do`" };

        match style {
            "start_of_block" => {
                // closing must align with do/{-line indent (first non-ws on that line)
                if end_col != start_of_line_indent {
                    diagnostics.push(self.diagnostic(
                        source,
                        end_line,
                        end_col,
                        format!("Align {} with {}.", close_word, open_word),
                    ));
                }
            }
            "start_of_line" => {
                // closing must align with start of the expression
                if end_col != expression_start_col && end_col != expression_start_indent {
                    diagnostics.push(self.diagnostic(
                        source,
                        end_line,
                        end_col,
                        format!(
                            "Align {} with the start of the line where the block is defined.",
                            close_word
                        ),
                    ));
                }
            }
            _ => {
                // "either" (default): accept alignment with:
                // - the do-line indent (start_of_block target), OR
                // - the expression start column (start_of_line target — from CallNode
                //   or assignment LHS), OR
                // - the expression start line indent, OR
                // - the CallNode start column (when the block closer is chained, i.e.,
                //   end/} is followed by .method — RuboCop's ancestor walk stops when
                //   the parent is on a different line, so the alignment target becomes
                //   the CallNode itself rather than the outermost assignment), OR
                // - the call expression column on the do-line (for hash-value blocks), OR
                // - the same-line operator LHS column (for &&/||/<< before call on same line)
                //
                // NOTE: do_col (the column of the `do`/`{` keyword itself) is NOT a
                // valid alignment target. RuboCop only accepts the indent of the do-line
                // (start_of_line_indent) or the expression start column, not the do column.
                //
                // NOTE: Cross-line operator continuations (||/&& on previous line) are NOT
                // valid alignment targets. RuboCop's `disqualified_parent?` stops the
                // ancestry walk when the parent is on a different line (except for masgn).
                // Accept call_start_col as an extra target only when the block is on the
                // RHS of an assignment and RuboCop would stop its ancestor walk before the
                // assignment target. That happens for:
                // - rescue modifier wrappers: `result = foo { ... } rescue false`
                // - safe-navigation chains: `result = foo { ... }&.path`
                // - chained calls that immediately open another block:
                //   `result = foo { ... }.check do ... end`
                //
                // Plain chained calls like `result = foo { ... }.to_json` do NOT qualify:
                // RuboCop walks through the normal send node to the assignment and aligns
                // the closer with the LHS.
                // Only accept expression_start_indent when the call actually starts
                // at the line's indent position (i.e., the call is the first thing on
                // the line). When the call is mid-line (e.g., inside parens like
                // `expect(auditable.audit(...) do`), the line indent is just the outer
                // context's indentation and shouldn't be a valid alignment target.
                let call_starts_at_indent = call_start_col == expression_start_indent;
                if end_col != start_of_line_indent
                    && end_col != expression_start_col
                    && (!call_starts_at_indent || end_col != expression_start_indent)
                    && (!accept_call_start || end_col != call_start_col)
                    && (!accept_call_expr_col || end_col != call_expr_col)
                    && same_line_operator_col.is_none_or(|c| end_col != c)
                {
                    diagnostics.push(self.diagnostic(
                        source,
                        end_line,
                        end_col,
                        format!(
                            "Align {} with the start of the line where the block is defined.",
                            close_word
                        ),
                    ));
                }
            }
        }
    }

    /// Check alignment for lambda/proc blocks (`-> { }` or `-> do end`).
    /// LambdaNode has opening_loc/closing_loc like BlockNode but is its own node type.
    fn check_lambda_alignment(
        &self,
        source: &SourceFile,
        lambda_node: &ruby_prism::LambdaNode<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let style = config.get_str("EnforcedStyleAlignWith", "either");

        let closing_loc = lambda_node.closing_loc();
        let closing_slice = closing_loc.as_slice();
        let is_do_end = closing_slice == b"end";
        let is_brace = closing_slice == b"}";
        if !is_do_end && !is_brace {
            return;
        }

        if !begins_its_line(source, closing_loc.start_offset()) {
            return;
        }
        let bytes = source.as_bytes();

        let opening_loc = lambda_node.opening_loc();
        let (opening_line, _) = source.offset_to_line_col(opening_loc.start_offset());

        let start_of_line_indent = line_indent(bytes, opening_loc.start_offset());

        // Use the `->` operator itself. With wrapped lambda arguments, the node's
        // overall span can begin after the stabby operator.
        let lambda_start_offset = lambda_node.operator_loc().start_offset();
        let (_, lambda_start_col) = source.offset_to_line_col(lambda_start_offset);

        let assignment_col = find_assignment_lhs_col(bytes, lambda_start_offset)
            .filter(|_| assignment_alignment_allowed(parse_result, &lambda_node.as_node()));
        let same_line_operator_col = find_same_line_operator(bytes, opening_loc.start_offset())
            .or_else(|| find_same_line_operator(bytes, lambda_start_offset))
            .map(|(col, _)| col);
        let expression_start_col = same_line_operator_col
            .or(assignment_col)
            .unwrap_or(lambda_start_col);
        let expression_start_indent = line_indent(bytes, lambda_start_offset);

        let (end_line, end_col) = source.offset_to_line_col(closing_loc.start_offset());

        if end_line == opening_line {
            return;
        }

        let close_word = if is_brace { "`}`" } else { "`end`" };
        let open_word = if is_brace { "`{`" } else { "`do`" };

        match style {
            "start_of_block" => {
                if end_col != start_of_line_indent {
                    diagnostics.push(self.diagnostic(
                        source,
                        end_line,
                        end_col,
                        format!("Align {} with {}.", close_word, open_word),
                    ));
                }
            }
            "start_of_line" => {
                if end_col != expression_start_col && end_col != expression_start_indent {
                    diagnostics.push(self.diagnostic(
                        source,
                        end_line,
                        end_col,
                        format!(
                            "Align {} with the start of the line where the block is defined.",
                            close_word
                        ),
                    ));
                }
            }
            _ => {
                // "either": accept alignment with do-line indent,
                // expression start col, the lambda start col, or the expression
                // start line indent only when the lambda actually starts there.
                // NOTE: do_col (column of `{`/`do`) is NOT a valid target.
                // NOTE: call_expr_col is NOT used for lambdas — the backward walk
                // from `{`/`do` gives the `->` position, not a meaningful call expr.
                let lambda_starts_at_indent = lambda_start_col == expression_start_indent;
                if end_col != start_of_line_indent
                    && end_col != expression_start_col
                    && (!lambda_starts_at_indent || end_col != expression_start_indent)
                    && end_col != lambda_start_col
                {
                    diagnostics.push(self.diagnostic(
                        source,
                        end_line,
                        end_col,
                        format!(
                            "Align {} with the start of the line where the block is defined.",
                            close_word
                        ),
                    ));
                }
            }
        }
    }
}

/// Get the indentation (number of leading whitespace characters) for the line
/// containing the given byte offset. Counts both spaces and tabs as 1 character
/// each to match `offset_to_line_col` which uses character (codepoint) offsets.
fn line_indent(bytes: &[u8], offset: usize) -> usize {
    let mut line_start = offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let mut indent = 0;
    while line_start + indent < bytes.len()
        && (bytes[line_start + indent] == b' ' || bytes[line_start + indent] == b'\t')
    {
        indent += 1;
    }
    indent
}

fn assignment_alignment_allowed(
    parse_result: &ruby_prism::ParseResult<'_>,
    node: &ruby_prism::Node<'_>,
) -> bool {
    struct Finder<'pr> {
        target_span: NodeSpan,
        target_tag: u8,
        stack: Vec<ruby_prism::Node<'pr>>,
        allowed: Option<bool>,
    }

    impl<'pr> Finder<'pr> {
        fn matches_target(&self, node: &ruby_prism::Node<'pr>) -> bool {
            node_type_tag(node) == self.target_tag && NodeSpan::from_node(node) == self.target_span
        }

        fn inspect_target(&self, node: &ruby_prism::Node<'pr>) -> bool {
            let mut current_span = NodeSpan::from_node(node);

            for parent in self.stack.iter().rev() {
                if alignment_assignment_parent(parent) {
                    return true;
                }

                if !transparent_alignment_parent(parent, current_span) {
                    return false;
                }

                current_span = NodeSpan::from_node(parent);
            }

            false
        }
    }

    impl<'pr> ruby_prism::Visit<'pr> for Finder<'pr> {
        fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
            if self.allowed.is_none() && self.matches_target(&node) {
                self.allowed = Some(self.inspect_target(&node));
            }

            self.stack.push(node);
        }

        fn visit_branch_node_leave(&mut self) {
            self.stack.pop();
        }

        fn visit_leaf_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
            if self.allowed.is_none() && self.matches_target(&node) {
                self.allowed = Some(self.inspect_target(&node));
            }
        }
    }

    let mut finder = Finder {
        target_span: NodeSpan::from_node(node),
        target_tag: node_type_tag(node),
        stack: Vec::new(),
        allowed: None,
    };
    ruby_prism::Visit::visit(&mut finder, &parse_result.node());
    finder.allowed.unwrap_or(false)
}

fn alignment_assignment_parent(node: &ruby_prism::Node<'_>) -> bool {
    is_assignment_type(node_type_tag(node))
        || node
            .as_call_node()
            .is_some_and(|call| call.equal_loc().is_some())
        || node.as_call_or_write_node().is_some()
        || node.as_call_and_write_node().is_some()
        || node.as_call_operator_write_node().is_some()
        || node.as_index_or_write_node().is_some()
        || node.as_index_and_write_node().is_some()
        || node.as_index_operator_write_node().is_some()
}

fn transparent_alignment_parent(parent: &ruby_prism::Node<'_>, current_span: NodeSpan) -> bool {
    if let Some(call) = parent.as_call_node() {
        return call
            .receiver()
            .is_some_and(|receiver| NodeSpan::from_node(&receiver) == current_span);
    }

    matches!(
        node_type_tag(parent),
        BEGIN_NODE | PARENTHESES_NODE | RESCUE_MODIFIER_NODE | STATEMENTS_NODE
    )
}

/// Check if the call expression at `call_start_offset` is the RHS of an assignment.
/// If so, return the column of the LHS variable (the assignment target).
/// This matches RuboCop's `find_lhs_node` which walks through op_asgn/masgn nodes.
///
fn find_assignment_lhs_col(bytes: &[u8], call_start_offset: usize) -> Option<usize> {
    if in_parameter_default_context(bytes, call_start_offset) {
        return None;
    }

    let mut current_line_start = line_start(bytes, call_start_offset);
    let mut current_line_end = line_end(bytes, current_line_start);

    if let Some(col) =
        find_assignment_lhs_col_on_line(bytes, current_line_start, current_line_end, false)
    {
        return Some(col);
    }

    while let Some(prev_line_start) = previous_line_start(bytes, current_line_start) {
        let prev_line_end = current_line_start - 1;
        let current_starts_continuation =
            line_starts_with_continuation(bytes, current_line_start, current_line_end);
        let prev_continues = trimmed_line_last_byte(bytes, prev_line_start, prev_line_end)
            .is_some_and(|b| matches!(b, b'.' | b',' | b'(' | b'[' | b'\\' | b'='));

        if !current_starts_continuation && !prev_continues {
            break;
        }

        current_line_start = prev_line_start;
        current_line_end = prev_line_end;

        if let Some(col) =
            find_assignment_lhs_col_on_line(bytes, current_line_start, current_line_end, true)
        {
            return Some(col);
        }
    }

    None
}

/// Walk backward from the `do` keyword on the same line to find the column where
/// the call expression starts. This handles cases like:
///   key: value.map do |x|
///        ^--- call_expr_col (aligned with value.map)
/// Returns the column of the first character of the call expression.
fn find_call_expression_col(bytes: &[u8], do_offset: usize) -> usize {
    // Find start of current line
    let mut line_start = do_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    // Walk backward from `do` to skip whitespace before it
    let mut pos = do_offset;
    while pos > line_start && bytes[pos - 1] == b' ' {
        pos -= 1;
    }

    // Now walk backward through the call expression.
    // We need to handle balanced parens/brackets/braces and stop at
    // unbalanced delimiters or spaces not inside nested structures.
    let mut paren_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    while pos > line_start {
        let ch = bytes[pos - 1];
        match ch {
            b')' | b']' => {
                paren_depth += 1;
                pos -= 1;
            }
            b'}' => {
                brace_depth += 1;
                pos -= 1;
            }
            b'(' | b'[' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                    pos -= 1;
                } else {
                    break;
                }
            }
            b'{' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                    pos -= 1;
                } else {
                    break;
                }
            }
            _ if paren_depth > 0 || brace_depth > 0 => {
                pos -= 1;
            } // inside parens, eat everything
            _ if ch.is_ascii_alphanumeric()
                || ch == b'_'
                || ch == b'.'
                || ch == b'?'
                || ch == b'!'
                || ch == b'@'
                || ch == b'$'
                || ch == b'%' =>
            {
                pos -= 1;
            }
            // `::` namespace separator
            b':' if pos >= 2 + line_start && bytes[pos - 2] == b':' => {
                pos -= 2;
            }
            _ => break,
        }
    }

    pos - line_start
}

/// If `pos` points just after a call expression and there's an assignment
/// operator (=, +=, -=, *=, /=, <<=, >>=, etc.) before it,
/// skip backward through the operator and whitespace, then walk backward
/// through the LHS identifier to find the assignment target.
/// Returns the new position (start of LHS), or `pos` unchanged if no
/// assignment is found.
fn skip_assignment_backward(bytes: &[u8], line_start: usize, pos: usize) -> usize {
    // Skip whitespace before the call expression
    let mut p = pos;
    while p > line_start && bytes[p - 1] == b' ' {
        p -= 1;
    }

    // Check for assignment operator ending with '='
    if p > line_start && bytes[p - 1] == b'=' {
        // Could be =, +=, -=, *=, /=, ||=, &&=, <<=, >>=, %=, **=, ^=
        // But NOT ==, !=, <=, >=
        let eq_pos = p - 1;
        let mut op_start = eq_pos;

        if op_start > line_start {
            let prev = bytes[op_start - 1];
            match prev {
                b'+' | b'-' | b'/' | b'%' | b'^' => {
                    op_start -= 1;
                }
                b'*' => {
                    // Could be *= or **=
                    op_start -= 1;
                    if op_start > line_start && bytes[op_start - 1] == b'*' {
                        op_start -= 1; // **=
                    }
                }
                b'|' => {
                    if op_start >= 2 + line_start && bytes[op_start - 2] == b'|' {
                        op_start -= 2; // ||=
                    } else {
                        op_start -= 1; // |=
                    }
                }
                b'&' => {
                    if op_start >= 2 + line_start && bytes[op_start - 2] == b'&' {
                        op_start -= 2; // &&=
                    } else {
                        op_start -= 1; // &=
                    }
                }
                b'<' if op_start >= 2 + line_start && bytes[op_start - 2] == b'<' => {
                    op_start -= 2;
                }
                b'>' if op_start >= 2 + line_start && bytes[op_start - 2] == b'>' => {
                    op_start -= 2;
                }
                // Bare `=` — but reject `==`, `!=`, `<=`, `>=`
                b'=' | b'!' | b'<' | b'>' => {
                    return pos; // Not a simple assignment
                }
                _ => {
                    // Bare `=` with a non-operator char before it — this is a simple assignment
                }
            }
        }

        // Skip whitespace before the operator
        let mut lhs_end = op_start;
        while lhs_end > line_start && bytes[lhs_end - 1] == b' ' {
            lhs_end -= 1;
        }

        // Walk backward through the LHS identifier (variable, ivar, cvar, etc.)
        // Handles balanced parens/brackets for complex LHS like:
        //   RequestStore.store[key(work_package)] = ...
        //   env[:machine].id = ...
        let mut lhs_pos = lhs_end;
        let mut lhs_paren_depth: i32 = 0;
        while lhs_pos > line_start {
            let ch = bytes[lhs_pos - 1];
            if lhs_paren_depth > 0 {
                // Inside balanced parens/brackets, eat everything
                match ch {
                    b')' | b']' => {
                        lhs_paren_depth += 1;
                        lhs_pos -= 1;
                    }
                    b'(' | b'[' => {
                        lhs_paren_depth -= 1;
                        lhs_pos -= 1;
                    }
                    _ => {
                        lhs_pos -= 1;
                    }
                }
            } else if ch == b')' || ch == b']' {
                lhs_paren_depth += 1;
                lhs_pos -= 1;
            } else if ch.is_ascii_alphanumeric()
                || ch == b'_'
                || ch == b'@'
                || ch == b'$'
                || ch == b'.'
                || ch == b'['
            {
                lhs_pos -= 1;
            } else if ch == b':' {
                // Single `:` for symbol keys inside brackets (e.g., `[:machine]`)
                // or `::` namespace separator
                lhs_pos -= 1;
                if lhs_pos > line_start && bytes[lhs_pos - 1] == b':' {
                    lhs_pos -= 1; // `::`
                }
            } else if ch == b',' {
                // Multi-assignment: `a, b = ...` — continue to find the first variable
                lhs_pos -= 1;
                while lhs_pos > line_start && bytes[lhs_pos - 1] == b' ' {
                    lhs_pos -= 1;
                }
            } else if ch == b' ' || ch == b'\t' {
                let mut gap_start = lhs_pos;
                while gap_start > line_start
                    && (bytes[gap_start - 1] == b' ' || bytes[gap_start - 1] == b'\t')
                {
                    gap_start -= 1;
                }

                if gap_start > line_start && bytes[gap_start - 1] == b',' {
                    lhs_pos = gap_start - 1;
                    while lhs_pos > line_start && bytes[lhs_pos - 1] == b' ' {
                        lhs_pos -= 1;
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        if lhs_pos < lhs_end {
            return lhs_pos;
        }
    }

    pos
}

/// Check if the block call on this line is immediately wrapped in a splat:
///   wrap *items.map { |item| ... }
///         ^
/// When RuboCop's ancestor walk stops at `splat`, the closing `}` must align with
/// the `*`, not with the call receiver.
fn find_same_line_splat_col(bytes: &[u8], call_start_offset: usize) -> Option<usize> {
    let mut line_start = call_start_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    if call_start_offset > line_start && bytes[call_start_offset - 1] == b'*' {
        return Some(call_start_offset - 1 - line_start);
    }

    None
}

fn chained_call_followed_by_index(bytes: &[u8], closer_offset: usize, closer_len: usize) -> bool {
    let mut pos = closer_offset + closer_len;
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t' || bytes[pos] == b'\r') {
        pos += 1;
    }

    while pos < bytes.len() && bytes[pos] != b'\n' {
        match bytes[pos] {
            b'[' => return true,
            b'.' => {
                pos += 1;
            }
            _ => {
                pos += 1;
            }
        }
    }

    false
}

/// When the block sits on the RHS of an assignment, accept the inner call start
/// as an alternate target only for the cases where RuboCop's ancestor walk stops
/// before reaching the assignment node.
fn accept_intermediate_call_start(bytes: &[u8], closer_offset: usize, closer_len: usize) -> bool {
    let after = closer_offset + closer_len;
    if after >= bytes.len() {
        return false;
    }

    let mut pos = after;
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t' || bytes[pos] == b'\r') {
        pos += 1;
    }

    if pos < bytes.len() && bytes[pos] != b'\n' {
        if bytes[pos] == b'&' && pos + 1 < bytes.len() && bytes[pos + 1] == b'.' {
            // Safe-navigation (`&.`) is a csend in RuboCop, so the ancestor walk
            // stops at the current block rather than walking through to assignment.
            return true;
        }
        if keyword_at(bytes, pos, b"rescue") {
            return true;
        }
        if bytes[pos] == b'.' {
            return chained_call_opens_block(bytes, pos + 1);
        }
        return false;
    }

    while pos < bytes.len() {
        if bytes[pos] == b'\n' {
            pos += 1;
        }
        while pos < bytes.len()
            && (bytes[pos] == b' ' || bytes[pos] == b'\t' || bytes[pos] == b'\r')
        {
            pos += 1;
        }
        if pos >= bytes.len() {
            return false;
        }
        if bytes[pos] == b'\n' {
            continue;
        }
        if bytes[pos] == b'&' && pos + 1 < bytes.len() && bytes[pos + 1] == b'.' {
            return true;
        }
        if bytes[pos] == b'.' {
            return chained_call_opens_block(bytes, pos + 1);
        }
        return false;
    }

    false
}

fn keyword_at(bytes: &[u8], pos: usize, keyword: &[u8]) -> bool {
    let Some(rest) = bytes.get(pos..) else {
        return false;
    };
    if !rest.starts_with(keyword) {
        return false;
    }

    let before_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric() && bytes[pos - 1] != b'_';
    let after_pos = pos + keyword.len();
    let after_ok = after_pos >= bytes.len()
        || (!bytes[after_pos].is_ascii_alphanumeric() && bytes[after_pos] != b'_');
    before_ok && after_ok
}

fn chained_call_opens_block(bytes: &[u8], mut pos: usize) -> bool {
    while pos < bytes.len() && bytes[pos] != b'\n' {
        if bytes[pos] == b'{' {
            return true;
        }
        if keyword_at(bytes, pos, b"do") {
            return true;
        }
        pos += 1;
    }
    false
}

/// Check if there's a `&&`, `||`, or `<<` operator on the same line before the
/// block opener (`do`/`{`). If so, return the column of the expression start on
/// the LHS of that operator. This handles patterns like:
///   next true if urls&.size&.positive? && urls&.all? do |url|
///   if adjustment_type == "removal" && article.tag_list.none? do |tag|
///   acc << items.map do |item|
///   lists << tag.ul(:class => "foo") do
///
/// Returns the column of the first non-whitespace token of the LHS expression.
fn find_same_line_operator_lhs(bytes: &[u8], opener_offset: usize) -> Option<usize> {
    find_same_line_operator(bytes, opener_offset).map(|(col, _)| col)
}

fn find_same_line_operator(
    bytes: &[u8],
    opener_offset: usize,
) -> Option<(usize, SameLineOperatorKind)> {
    let mut line_start = opener_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    // Walk backward from the block opener to the start of the immediate call
    // expression on this line, then check whether that call is wrapped by
    // &&, ||, or << on the same line.
    let mut pos = opener_offset;

    // Skip whitespace before `do` / `{`
    while pos > line_start && bytes[pos - 1] == b' ' {
        pos -= 1;
    }

    let mut paren_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    while pos > line_start {
        let ch = bytes[pos - 1];
        match ch {
            b')' | b']' => {
                paren_depth += 1;
                pos -= 1;
            }
            b'}' => {
                brace_depth += 1;
                pos -= 1;
            }
            b'(' | b'[' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                    pos -= 1;
                } else {
                    break;
                }
            }
            b'{' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                    pos -= 1;
                } else {
                    break;
                }
            }
            _ if paren_depth > 0 || brace_depth > 0 => {
                pos -= 1;
            }
            _ if ch.is_ascii_alphanumeric()
                || ch == b'_'
                || ch == b'.'
                || ch == b'?'
                || ch == b'!'
                || ch == b'@'
                || ch == b'$'
                || ch == b'%' =>
            {
                pos -= 1;
            }
            b':' if pos >= 2 + line_start && bytes[pos - 2] == b':' => {
                pos -= 2;
            }
            _ => break,
        }
    }

    while pos > line_start && bytes[pos - 1] == b' ' {
        pos -= 1;
    }

    // Check for &&, ||, or << immediately before the call expression
    if pos >= 2 + line_start {
        let op1 = bytes[pos - 2];
        let op2 = bytes[pos - 1];
        let kind = if op1 == b'&' && op2 == b'&' {
            Some(SameLineOperatorKind::Logical)
        } else if op1 == b'|' && op2 == b'|' {
            Some(SameLineOperatorKind::Logical)
        } else if op1 == b'<' && op2 == b'<' {
            Some(SameLineOperatorKind::Shovel)
        } else {
            None
        };

        if let Some(kind) = kind {
            pos -= 2;
            // Skip whitespace before the operator
            while pos > line_start && bytes[pos - 1] == b' ' {
                pos -= 1;
            }
            // Walk backward through the LHS expression to find its start.
            // The LHS can contain any Ruby expression (strings, comparisons, etc.),
            // so we use a permissive walk that handles balanced parens/brackets/quotes
            // and stops only at unbalanced open delimiters or line start.
            let lhs_end = pos;
            let mut paren_depth: i32 = 0;
            while pos > line_start {
                let ch = bytes[pos - 1];
                match ch {
                    b')' | b']' => {
                        paren_depth += 1;
                        pos -= 1;
                    }
                    b'(' | b'[' => {
                        if paren_depth > 0 {
                            paren_depth -= 1;
                            pos -= 1;
                        } else {
                            break;
                        }
                    }
                    _ if paren_depth > 0 => {
                        pos -= 1;
                    }
                    // Walk through string literals (balanced quotes)
                    b'"' => {
                        pos -= 1; // skip closing quote
                        while pos > line_start && bytes[pos - 1] != b'"' {
                            pos -= 1;
                        }
                        if pos > line_start {
                            pos -= 1; // skip opening quote
                        }
                    }
                    b'\'' => {
                        pos -= 1;
                        while pos > line_start && bytes[pos - 1] != b'\'' {
                            pos -= 1;
                        }
                        if pos > line_start {
                            pos -= 1;
                        }
                    }
                    // Accept most non-whitespace characters in the LHS expression
                    _ if ch != b' ' && ch != b'\t' => {
                        pos -= 1;
                    }
                    // Stop at whitespace — but only if the next non-ws char before
                    // it is not a keyword/identifier continuation
                    _ => {
                        // Skip this whitespace gap
                        let gap_end = pos;
                        while pos > line_start
                            && (bytes[pos - 1] == b' ' || bytes[pos - 1] == b'\t')
                        {
                            pos -= 1;
                        }
                        // If we reached line start or an unbalanced open paren, stop
                        if pos == line_start {
                            break;
                        }
                        // Check what's before the gap — if it's a keyword like `if`, `unless`, etc.
                        // stop here (the LHS starts after the keyword)
                        let before_gap = &bytes[line_start..pos];
                        if before_gap.ends_with(b"if")
                            || before_gap.ends_with(b"unless")
                            || before_gap.ends_with(b"while")
                            || before_gap.ends_with(b"until")
                            || before_gap.ends_with(b"return")
                        {
                            // Check it's a keyword (preceded by space or line start)
                            let kw_len = if before_gap.ends_with(b"unless")
                                || before_gap.ends_with(b"return")
                            {
                                6
                            } else if before_gap.ends_with(b"while")
                                || before_gap.ends_with(b"until")
                            {
                                5
                            } else {
                                2
                            };
                            let kw_start = pos - kw_len;
                            if kw_start == line_start
                                || bytes[kw_start - 1] == b' '
                                || bytes[kw_start - 1] == b'\t'
                            {
                                pos = gap_end;
                                break;
                            }
                        }
                        // Otherwise continue walking through the gap
                    }
                }
            }
            // Skip any leading whitespace to get to the first non-ws character
            while pos < lhs_end && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
                pos += 1;
            }
            if pos < lhs_end {
                if starts_with_line_leading_closer(bytes, pos, lhs_end) {
                    return None;
                }
                return Some((pos - line_start, kind));
            }
        }
    }

    None
}

fn starts_with_line_leading_closer(bytes: &[u8], start: usize, end: usize) -> bool {
    if start >= end {
        return false;
    }

    match bytes[start] {
        b')' | b']' | b'}' => true,
        b'e' => keyword_at(bytes, start, b"end"),
        _ => false,
    }
}

fn line_start(bytes: &[u8], offset: usize) -> usize {
    let mut line_start = offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    line_start
}

fn line_end(bytes: &[u8], line_start: usize) -> usize {
    let mut line_end = line_start;
    while line_end < bytes.len() && bytes[line_end] != b'\n' {
        line_end += 1;
    }
    line_end
}

fn previous_line_start(bytes: &[u8], line_start: usize) -> Option<usize> {
    if line_start == 0 {
        return None;
    }

    let mut prev_line_start = line_start - 1;
    while prev_line_start > 0 && bytes[prev_line_start - 1] != b'\n' {
        prev_line_start -= 1;
    }
    Some(prev_line_start)
}

fn first_non_ws(bytes: &[u8], line_start: usize, line_end: usize) -> Option<usize> {
    let mut pos = line_start;
    while pos < line_end && (bytes[pos] == b' ' || bytes[pos] == b'\t' || bytes[pos] == b'\r') {
        pos += 1;
    }
    (pos < line_end).then_some(pos)
}

fn trimmed_line_last_byte(bytes: &[u8], line_start: usize, line_end: usize) -> Option<u8> {
    let mut pos = line_end;
    while pos > line_start && matches!(bytes[pos - 1], b' ' | b'\t' | b'\r') {
        pos -= 1;
    }
    (pos > line_start).then_some(bytes[pos - 1])
}

fn line_starts_with_continuation(bytes: &[u8], line_start: usize, line_end: usize) -> bool {
    let Some(pos) = first_non_ws(bytes, line_start, line_end) else {
        return false;
    };

    bytes[pos] == b'.' || (bytes[pos] == b'&' && pos + 1 < line_end && bytes[pos + 1] == b'.')
}

fn rightmost_assignment_eq(bytes: &[u8], line_start: usize, line_end: usize) -> Option<usize> {
    let mut pos = line_end;
    while pos > line_start {
        pos -= 1;
        if bytes[pos] != b'=' {
            continue;
        }

        let prev = (pos > line_start).then_some(bytes[pos - 1]);
        let next = (pos + 1 < line_end).then_some(bytes[pos + 1]);
        if matches!(prev, Some(b'=' | b'!' | b'<' | b'>')) || next == Some(b'~') {
            continue;
        }

        return Some(pos);
    }

    None
}

fn find_assignment_lhs_col_on_line(
    bytes: &[u8],
    line_start: usize,
    line_end: usize,
    allow_empty_rhs: bool,
) -> Option<usize> {
    let eq_pos = rightmost_assignment_eq(bytes, line_start, line_end)?;

    let mut rhs_start = eq_pos + 1;
    while rhs_start < line_end
        && (bytes[rhs_start] == b' ' || bytes[rhs_start] == b'\t' || bytes[rhs_start] == b'\r')
    {
        rhs_start += 1;
    }

    if rhs_start < line_end {
        let lhs_start = skip_assignment_backward(bytes, line_start, rhs_start);
        return (lhs_start != rhs_start).then_some(lhs_start - line_start);
    }

    if !allow_empty_rhs {
        return None;
    }

    let lhs_portion = &bytes[line_start..eq_pos];
    if !lhs_portion.contains(&b',') {
        return None;
    }

    lhs_portion
        .iter()
        .position(|&b| b != b' ' && b != b'\t' && b != b'\r')
}

fn in_parameter_default_context(bytes: &[u8], offset: usize) -> bool {
    let mut pos = offset;
    let mut paren_depth: usize = 0;

    while pos > 0 {
        pos -= 1;
        match bytes[pos] {
            b')' => paren_depth += 1,
            b'(' => {
                if paren_depth == 0 {
                    let line_start = line_start(bytes, pos);
                    let mut prefix = &bytes[line_start..pos];

                    while let Some(last) = prefix.last() {
                        if matches!(last, b' ' | b'\t' | b'\r') {
                            prefix = &prefix[..prefix.len() - 1];
                        } else {
                            break;
                        }
                    }

                    let mut trimmed_start = 0;
                    while trimmed_start < prefix.len()
                        && matches!(prefix[trimmed_start], b' ' | b'\t' | b'\r')
                    {
                        trimmed_start += 1;
                    }
                    let trimmed = &prefix[trimmed_start..];

                    return trimmed.starts_with(b"def ")
                        || trimmed.starts_with(b"defs ")
                        || trimmed.ends_with(b"->");
                }
                paren_depth -= 1;
            }
            _ => {}
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(BlockAlignment, "cops/layout/block_alignment");

    #[test]
    fn brace_block_no_offense() {
        let source = b"items.each { |x|\n  puts x\n}\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn start_of_block_style() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("start_of_block".into()),
            )]),
            ..CopConfig::default()
        };
        // In start_of_block style, `end` must align with the do-line indent
        // (first non-ws on the do-line), not the `do` keyword column.
        // For `items.each do |x|`, do-line indent = 0, so end at col 0 is fine.
        let src = b"items.each do |x|\n  puts x\nend\n";
        let diags = run_cop_full_with_config(&BlockAlignment, src, config.clone());
        assert!(
            diags.is_empty(),
            "start_of_block: end at col 0 matches do-line indent 0. Got: {:?}",
            diags
        );

        // But end at col 2 should be flagged (doesn't match do-line indent 0)
        let src2 = b"items.each do |x|\n  puts x\n  end\n";
        let diags2 = run_cop_full_with_config(&BlockAlignment, src2, config.clone());
        assert_eq!(
            diags2.len(),
            1,
            "start_of_block should flag end at col 2 (doesn't match do-line indent 0)"
        );

        // Chained: .each do at col 2, end should align at col 2
        let src3 = b"foo.bar\n  .each do\n    baz\n  end\n";
        let diags3 = run_cop_full_with_config(&BlockAlignment, src3, config.clone());
        assert!(
            diags3.is_empty(),
            "start_of_block: end at col 2 matches .each do line indent. Got: {:?}",
            diags3
        );

        // Chained: .each do at col 2, end at col 0 should flag
        let src4 = b"foo.bar\n  .each do\n    baz\nend\n";
        let diags4 = run_cop_full_with_config(&BlockAlignment, src4, config);
        assert_eq!(
            diags4.len(),
            1,
            "start_of_block: end at col 0 doesn't match .each do line indent 2"
        );
    }

    // FP fix: trailing-dot method chains
    #[test]
    fn no_offense_trailing_dot_chain() {
        let source =
            b"all_objects.flat_map { |o| o }.\n  uniq(&:first).each do |a, o|\n  process(a, o)\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Trailing dot chain: end should align with chain root. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_trailing_dot_chain_indented() {
        let source = b"def foo\n  objects.flat_map { |o| o }.\n    uniq.each do |item|\n    process(item)\n  end\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Indented trailing dot chain: end at col 2 matches chain start at col 2. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_trailing_dot_multi_line() {
        let source = b"  records.\n    where(active: true).\n    order(:name).each do |r|\n    process(r)\n  end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Multi trailing dot: end at col 2 matches chain root at col 2. Got: {:?}",
            diags
        );
    }

    // FP fix: tab indentation
    #[test]
    fn no_offense_tab_indented_block() {
        let source = b"if true\n\titems.each do\n\t\tputs 'hello'\n\tend\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Tab-indented block should not be flagged. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_tab_indented_assignment_block() {
        let source = b"\tvariable = test do |x|\n\t\tx.to_s\n\tend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Tab-indented assignment block should not be flagged. Got: {:?}",
            diags
        );
    }

    // FP fix: begins_its_line check
    #[test]
    fn fp_end_not_beginning_its_line() {
        // end.select is at start of line (after whitespace) but has continuation
        // The first block's end should not be checked since it has .select after it
        let source = b"def foo(bar)\n  bar.get_stuffs\n      .reject do |stuff|\n        stuff.long_expr\n      end.select do |stuff|\n        stuff.other\n      end\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Should not flag end that doesn't begin its line. Got: {:?}",
            diags
        );
    }

    // FN fix: brace block misalignment
    #[test]
    fn offense_brace_block_misaligned() {
        let source = b"test {\n  stuff\n  }\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "Misaligned brace block should be flagged. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_brace_block_aligned() {
        let source = b"test {\n  stuff\n}\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Aligned brace block should not be flagged. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_brace_block_not_beginning_line() {
        let source = b"scope :bar, lambda { joins(:baz)\n                     .distinct }\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "closing brace not beginning its line should not be flagged"
        );
    }

    // Other patterns from RuboCop spec
    #[test]
    fn no_offense_variable_assignment() {
        let source = b"variable = test do |ala|\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "end aligned with variable start. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_op_asgn() {
        let source = b"rb += files.select do |file|\n  file << something\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(diags.is_empty(), "end aligned with rb. Got: {:?}", diags);
    }

    #[test]
    fn no_offense_logical_operand() {
        let source = b"(value.is_a? Array) && value.all? do |subvalue|\n  type_check_value(subvalue, array_type)\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "end aligns with expression start. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_send_shovel() {
        let source = b"parser.children << lambda do |token|\n  token << 1\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "end aligns with parser.children. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_chain_pretty_alignment() {
        let source = b"def foo(bar)\n  bar.get_stuffs\n      .reject do |stuff|\n        stuff.long_expr\n      end\n      .select do |stuff|\n        stuff.other\n      end\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "end at col 6 matches do-line indent. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_next_line_assignment() {
        let source = b"variable =\n  a_long_method do |v|\n    v.foo\n  end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "end aligns with a_long_method. Got: {:?}",
            diags
        );
    }

    // FP fix: string concatenation with + across lines (RSpec-style descriptions)
    #[test]
    fn no_offense_plus_continuation() {
        // it "something " + "else" do ... end
        let source = b"it \"should convert \" +\n    \"correctly\" do\n  run_test\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Plus continuation: end at col 0 matches chain root. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_plus_continuation_describe() {
        // describe with + continuation spanning 3 lines
        let source = b"describe User, \"when created \" +\n    \"with issues\" do\n  it \"works\" do\n    true\n  end\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Describe + continuation: end at col 0 matches describe. Got: {:?}",
            diags
        );
    }

    // FN fix: end aligns with RHS of assignment instead of LHS
    #[test]
    fn offense_end_aligns_with_rhs() {
        // answer = prompt.select do ... end — end should align with answer, not prompt
        let source =
            b"answer = prompt.select do |menu|\n           menu.choice \"A\"\n         end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "end at col 9 aligns with prompt (RHS) not answer (LHS). Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_assignment_end_aligns_with_lhs() {
        // answer = prompt.select do ... end — end at col 0 aligns with answer (LHS)
        let source = b"answer = prompt.select do |menu|\n  menu.choice \"A\"\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "end at col 0 matches answer (LHS). Got: {:?}",
            diags
        );
    }

    // Ensure hash value blocks still work (not regressed by assignment fix)
    #[test]
    fn no_offense_hash_value_block() {
        let source = b"def generate\n  {\n    data: items.map do |item|\n            item.to_s\n          end,\n  }\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Hash value: end aligns with items.map. Got: {:?}",
            diags
        );
    }

    // Block inside parentheses (like expect(...))
    #[test]
    fn no_offense_block_in_parens() {
        let source = b"expect(arr.all? do |o|\n         o.valid?\n       end)\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Block in parens: end at col 7 matches arr.all?. Got: {:?}",
            diags
        );
    }

    // FP fix: chained blocks with end aligning with method call (active_merchant)
    #[test]
    fn fp_chained_block_end_aligns_with_method() {
        // response = stub_comms do ... end.check_request do ... end.respond_with(...)
        // The first end at col 11 aligns with stub_comms at col 11
        let source = b"response = stub_comms do\n             @gateway.verify(@credit_card, @options)\n           end.check_request do |_endpoint, data, _headers|\n  assert_match(/pattern/, data)\nend.respond_with(response)\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Chained blocks: end at col 11 matches stub_comms. Got: {:?}",
            diags
        );
    }

    // Brace block } aligned with call start in chained context
    #[test]
    fn no_offense_brace_chained() {
        // } is followed by .sort_by (chained), so call_start_col is accepted
        let source = b"victims = replicas.select {\n            !(it.destroy_set?)\n          }.sort_by { |r| r.created_at }\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Chained brace: }} at col 10 matches call. Got: {:?}",
            diags
        );
    }

    // FN fix: Hash.new with block end misaligned (jruby)
    #[test]
    fn fn_hash_new_block_end_misaligned() {
        let source = b"NF_HASH_D = Hash.new do |hash, key|\n                       hash.shift if hash.length>MAX_HASH_LENGTH\n                       hash[key] = nfd_one(key)\n                     end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "Hash.new end at col 21 misaligned with NF_HASH_D at col 0. Got: {:?}",
            diags
        );
    }

    // FP: } followed by newline + .sort_by (chained via next-line dot)
    #[test]
    fn fp_brace_chained_next_line_dot() {
        // } at col 16, followed by \n        .sort_by
        // RuboCop accepts this — the block is chained
        let source = b"      victims = replicas.select {\n                  !(it.destroy_set? || it.strand.label == \"destroy\")\n                }\n        .sort_by { |r| r.created_at }\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "FP: brace chained via next-line dot should not be flagged. Got: {:?}",
            diags
        );
    }

    // Known remaining FP: } inside parenthesized expression with rescue modifier
    // (automaticmode__active_workflow, 1 FP). The } aligns with neither the
    // assignment LHS nor the call expression start. RuboCop accepts it through
    // AST parent walk that nitrocop can't replicate with byte-level heuristics.

    // Known remaining FP: } aligned with block body for splat method arg block
    // (flyerhzm__rails_best_practices, 1 FP). Deep indentation method arg pattern
    // where } aligns with the block body, not any standard alignment target.

    // FP: do..end block as part of if condition with &&
    #[test]
    fn fp_do_end_in_if_condition() {
        // if adjustment_type == "removal" && article.tag_list.none? do |tag|
        //      tag.casecmp(tag_name).zero?
        //    end
        let source = b"    if adjustment_type == \"removal\" && article.tag_list.none? do |tag|\n         tag.casecmp(tag_name).zero?\n       end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "FP: do..end in if condition with && should not be flagged. Got: {:?}",
            diags
        );
    }

    // FP: end at col 6 for .reduce block with multiline assignment on previous line
    #[test]
    fn fp_reduce_multiline_assignment() {
        let source = b"    def packages_lines(stdout)\n      packages_lines, last_package_lines =\n        stdout\n        .each_line\n        .map(&:strip)\n        .reject { |line| end_of_package_lines?(line) }\n        .reduce([[], []]) do |(packages_lines, package_lines), line|\n        if start_of_package_lines?(line)\n          packages_lines.push(package_lines) unless package_lines.empty?\n          [packages_lines, [line]]\n        else\n          package_lines.push(line)\n          [packages_lines, package_lines]\n        end\n      end\n\n      packages_lines.push(last_package_lines)\n    end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "FP: end at col 6 matches multiline assignment LHS. Got: {:?}",
            diags
        );
    }

    #[test]
    fn helper_multiline_chain_assignment_lhs_col() {
        struct Finder {
            offset: Option<usize>,
        }

        impl<'pr> ruby_prism::Visit<'pr> for Finder {
            fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
                if self.offset.is_none() && node.block().and_then(|b| b.as_block_node()).is_some() {
                    self.offset = Some(node.location().start_offset());
                }
                ruby_prism::visit_call_node(self, node);
            }
        }

        let source = b"changed_playlist, new, changed_position, unchanged = playlist.items.\n  sort_by(&:position).\n  group_by do |item|\n    item\nend.values_at(:changed_playlist, :new, :changed_position, :unchanged).map(&:to_a)\n";
        let parse = ruby_prism::parse(source);
        let mut finder = Finder { offset: None };
        ruby_prism::Visit::visit(&mut finder, &parse.node());
        let offset = finder.offset.expect("block call offset");
        assert_eq!(find_assignment_lhs_col(source, offset), Some(0));
    }

    #[test]
    fn helper_multiline_zip_map_assignment_lhs_col() {
        struct Finder {
            offset: Option<usize>,
        }

        impl<'pr> ruby_prism::Visit<'pr> for Finder {
            fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
                if self.offset.is_none() && node.block().and_then(|b| b.as_block_node()).is_some() {
                    self.offset = Some(node.location().start_offset());
                }
                ruby_prism::visit_call_node(self, node);
            }
        }

        let source = b"@fsa, @fsb, @fsc = create_list(:form_submission, 3, user_account: user_account)\n                   .zip(%w[FORM-A FORM-B FORM-C])\n                   .map do |submission, form_type|\n                     submission.update(form_type: form_type)\n                     submission\nend\n";
        let parse = ruby_prism::parse(source);
        let mut finder = Finder { offset: None };
        ruby_prism::Visit::visit(&mut finder, &parse.node());
        let offset = finder.offset.expect("block call offset");
        assert_eq!(find_assignment_lhs_col(source, offset), Some(0));
    }

    // FN: end misaligned in multi-arg call with do block (Arachni pattern)
    #[test]
    fn fn_end_misaligned_in_multi_arg_call() {
        // expect(auditable.audit( {},
        //                   format: [...]) do |_, element|
        //     injected << element.affected_input_value
        // end).to be_nil
        // end at col 24, but auditable.audit at col 31 and do-line indent ~42
        let source = b"                        expect(auditable.audit( {},\n                                          format: [ Format::STRAIGHT ] ) do |_, element|\n                            injected << element.affected_input_value\n                        end).to be_nil\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: end at col 24 misaligned in multi-arg call. Got: {:?}",
            diags
        );
    }

    // FN: } misaligned in brace block (seyhunak pattern)
    #[test]
    fn fn_brace_misaligned_deep_block() {
        // have_tag(:div,
        //   with: {class: "alert"}) {
        //     have_tag(:button, ...)
        //   }          <-- } at col 6, but call starts much deeper
        let source = b"      expect(element).to have_tag(:div,\n        with: {class: \"alert\"}) {\n          have_tag(:button,\n            text: \"x\"\n          )\n\n      }\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: brace at col 6 misaligned with have_tag at col 25. Got: {:?}",
            diags
        );
    }

    // FN: end misaligned off by 1 (randym pattern)
    #[test]
    fn fn_end_misaligned_by_one() {
        // %w(...).each do |attr|
        //    body
        //  end         <-- end at col 4, but %w at col 3
        let source = b"   %w(param1 param2).each do |attr|\n      assert_raise(ArgumentError) { @dn.send(attr) }\n    end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: end at col 4 misaligned with %w at col 3. Got: {:?}",
            diags
        );
    }

    // Known remaining FN: } misaligned in assignment context with chained closer
    // (diaspora, 1 FN). `json = bob.contacts.map { ... }.to_json` — the chained
    // `.to_json` causes nitrocop to accept call_start_col as alignment target.
    // RuboCop uses AST parent walk to resolve through the chain to the assignment.

    // FN: end misaligned in accepted_states.any? (sharetribe pattern)
    #[test]
    fn fn_end_misaligned_any_block() {
        let source = b"        accepted_states.any? do |(status, reason)|\n        if reason.nil?\n          payment[:payment_status] == status\n        end\n          end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: end at col 10 misaligned with accepted_states at col 8. Got: {:?}",
            diags
        );
    }

    // FN: end misaligned by 2 in Thread::new block (trogdoro pattern)
    #[test]
    fn fn_thread_new_block_misaligned() {
        let source = b"            Thread::new(iodat, main) do |iodat, main|\n              process(iodat)\n          end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: end at col 10 misaligned with Thread at col 12. Got: {:?}",
            diags
        );
    }

    // FN: end misaligned in combos block (bloom-lang pattern)
    #[test]
    fn fn_combos_block_misaligned() {
        let source = b"    result <= (sem_hist * use_tiebreak * explicit_tc).combos(sem_hist.from => use_tiebreak.from,\n                                                             sem_hist.to => explicit_tc.from,\n                                                             sem_hist.from => explicit_tc.to) do |s,t,e|\n      [s.to, t.to]\n    end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: end at col 4 misaligned with result or (sem_hist. Got: {:?}",
            diags
        );
    }

    // FN: } misaligned lambda block (refinery pattern)
    #[test]
    fn fn_lambda_brace_misaligned() {
        let source = b"  ->{\n    page.within_frame do\n      select_upload\n    end\n    }\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: brace at col 4 misaligned with -> at col 2. Got: {:?}",
            diags
        );
    }

    #[test]
    fn fn_same_line_or_wrapper_misaligned() {
        let source = b"def changed?\n  to_be_destroyed.any? || proxy_target.any? do |record|\n    record.changed?\n                          end\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: end should align with the || expression, not proxy_target.any?. Got: {:?}",
            diags
        );
    }

    #[test]
    fn fn_hash_receiver_each_block_misaligned() {
        let source = b"{\n  \"Ab$9\" => 4,\n  \"blah\" => -2\n}.each do |password, bonus_bits|\n  puts password\n end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: end should align with the hash receiver / }}.each line. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_hash_receiver_each_block_aligned() {
        let source = b"{\n  \"Ab$9\" => 4,\n  \"blah\" => -2\n}.each do |password, bonus_bits|\n  puts password\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Hash receiver chained into each should allow end at col 0. Got: {:?}",
            diags
        );
    }

    #[test]
    fn fn_splat_wrapper_block_misaligned() {
        let source = b"rdoc.rdoc_files.include(\n  *FileList.new(\"*\") do |list|\n     list.exclude(\"TODO\")\n   end.to_a)\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: end should align with the splat, not FileList. Got: {:?}",
            diags
        );
    }

    #[test]
    fn fn_shovel_wrapper_do_end_misaligned() {
        let source = b"out << sequence.each_with_object(+'') do |col_name, s|\n  s << col_name.to_s\n       end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: end should align with out <<, not sequence.each_with_object. Got: {:?}",
            diags
        );
    }

    #[test]
    fn fn_shovel_wrapper_brace_block_misaligned() {
        let source = b"def handle_message(msg, connection = {})\n  if request?(msg)\n    tp << ThreadPoolJob.new(msg) { |i|\n      handle_request(i, false, connection)\n          }\n  end\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: }} should align with tp <<, not ThreadPoolJob.new. Got: {:?}",
            diags
        );
    }

    #[test]
    fn fn_or_asgn_chain_block_misaligned() {
        let source = b"def link_options\n  @link_options ||= pages.published.pluck(:name, :slug)\n    .each_with_object(DEFAULT_LINKS.dup) do |(name, slug), memo|\n    memo[name] = slug\n  end.sort_by { |_key, value| navigation_links.index(value) || 0 }.to_h\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: ||= chain should not accept the assignment LHS here. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_or_asgn_chain_block_aligned() {
        let source = b"def link_options\n  @link_options ||= pages.published.pluck(:name, :slug)\n    .each_with_object(DEFAULT_LINKS.dup) do |(name, slug), memo|\n    memo[name] = slug\n    end.sort_by { |_key, value| navigation_links.index(value) || 0 }.to_h\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Correctly aligned ||= continuation chain should not be flagged. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_or_asgn_plain_block_aligned() {
        let source = b"result ||= items.map do |item|\n  item\nend\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Plain ||= memoization block should align with the assignment LHS. Got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_or_asgn_plain_chained_send_aligned() {
        let source = b"result ||= items.map do |item|\n  item\nend.to_json\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "Plain chained send after ||= should still allow the assignment LHS. Got: {:?}",
            diags
        );
    }

    #[test]
    fn fn_multiline_stabby_lambda_do_end_misaligned() {
        let source = b"          scope :_candlestick, -> (timeframe: '1h',\n                           segment_by: segment_by_column,\n                           time: time_column,\n                           volume: 'volume',\n                           value: value_column) do\n             select(time)\n          end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: multiline stabby lambda should align with -> or do-line indent. Got: {:?}",
            diags
        );
    }

    #[test]
    fn helper_finds_same_line_or_lhs() {
        let source = b"  to_be_destroyed.any? || proxy_target.any? do |record|\n";
        let opener = std::str::from_utf8(source)
            .unwrap()
            .find(" do |record|")
            .unwrap()
            + 1;
        assert_eq!(find_same_line_operator_lhs(source, opener), Some(2));
    }

    #[test]
    fn helper_finds_same_line_shovel_lhs() {
        let source = b"out << sequence.each_with_object(+'') do |col_name, s|\n";
        let opener = std::str::from_utf8(source)
            .unwrap()
            .find(" do |col_name, s|")
            .unwrap()
            + 1;
        assert_eq!(find_same_line_operator_lhs(source, opener), Some(0));
    }

    #[test]
    fn helper_finds_same_line_shovel_lhs_for_brace_block() {
        let source = b"    tp << ThreadPoolJob.new(msg) { |i|\n";
        let opener = std::str::from_utf8(source).unwrap().find("{ |i|").unwrap();
        assert_eq!(find_same_line_operator_lhs(source, opener), Some(4));
    }

    #[test]
    fn helper_ignores_same_line_operator_lhs_when_line_starts_with_closer() {
        let source = b"left_side.find {\n  it\n} || right_side.any? do |item|\n";
        let opener = std::str::from_utf8(source)
            .unwrap()
            .find(" do |item|")
            .unwrap()
            + 1;
        assert_eq!(find_same_line_operator_lhs(source, opener), None);
    }

    #[test]
    fn fp_rhs_block_after_line_leading_closer_or() {
        let source = b"left_side.find {\n  it\n} || right_side.any? do |item|\n  item\n     end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "FP: a line-leading closer before || should not hide the RHS block target. Got: {:?}",
            diags
        );
    }

    // FP: do on continuation line of multi-line method call with assignment
    // env[:machine].id = env[:machine].provider.driver.clone_vm(
    //   env[:clone_id], options) do |progress|
    //   ...
    // end   <-- end at col 10, aligns with assignment LHS env[:machine].id
    #[test]
    fn fp_do_on_continuation_line_with_assignment() {
        let source = b"          env[:machine].id = env[:machine].provider.driver.clone_vm(\n            env[:clone_id], options) do |progress|\n            env[:ui].clear_line\n          end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "FP: end at col 10 aligns with LHS env[:machine].id at col 10. Got: {:?}",
            diags
        );
    }

    // FP: do on continuation line of multi-line ask() call
    // entry[:phone] = ask("Phone?  ",
    //                     lambda { ... }) do |q|
    //   q.validate = ...
    // end   <-- end at col 2, aligns with entry[:phone] at col 2
    #[test]
    fn fp_do_on_continuation_line_ask() {
        let source = b"  entry[:phone] = ask(\"Phone?  \",\n                      lambda { |p| p.to_s }) do |q|\n    q.validate = true\n  end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "FP: end at col 2 aligns with entry[:phone] at col 2. Got: {:?}",
            diags
        );
    }

    // FP: do on continuation line with multi-line args (openstreetmap pattern)
    // lists << tag.ul(:class => [...]) do
    //   ...
    // end   <-- end at col 6, aligns with do-line indent or lists at col 6
    #[test]
    fn fp_multiline_args_tag_ul() {
        let source = b"      lists << tag.ul(:class => [\n                        \"pagination\",\n                      ]) do\n        items.each do |page|\n          concat page\n        end\n      end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "FP: end at col 6 aligns with lists at col 6. Got: {:?}",
            diags
        );
    }

    // FP: .select do on continuation line of chained call (openproject pattern)
    // custom_fields
    //   .select do |cf|
    //     cf.something
    // end   <-- end at col 6, aligns with custom_fields indent
    #[test]
    fn fp_select_do_continuation_chain() {
        let source = b"      RequestStore.store[key] = custom_fields\n                                   .select do |cf|\n        cf.available?\n      end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert!(
            diags.is_empty(),
            "FP: end at col 6 aligns with do-line indent or expression. Got: {:?}",
            diags
        );
    }

    // FN: end misaligned in %w[].each (floere pattern)
    #[test]
    fn fn_end_misaligned_each_block() {
        let source = b"%w[cpu object].each do |thing|\n  profile thing do\n    10_000.times { method }\n  end\n end\n";
        let diags = run_cop_full(&BlockAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "FN: end at col 1 misaligned with %w at col 0. Got: {:?}",
            diags
        );
    }
}
