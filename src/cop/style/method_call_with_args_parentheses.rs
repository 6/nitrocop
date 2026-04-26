use ruby_prism::Visit;

use crate::cop::shared::access_modifier_predicates::MacroScope;
use crate::cop::shared::{method_dispatch_predicates, method_identifier_predicates};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-15)
///
/// Corpus oracle reported FP=59, FN=54,201.
///
/// ### FP=59→0 (fixed)
/// Root cause: `visit_lambda_node` pushed `MacroScope::NotMacroScope`, breaking macro scope
/// inheritance. RuboCop's `macro?` returns true for calls inside lambdas in
/// class/module bodies. Fixed by using `wrapper_child_scope()` for lambdas.
///
/// ### FN=54,201→9,647 (44,554 fixed, ~9.6k remaining)
///
/// Fix 1 — YieldNode handling (commit 785468fe, ~13.2k FN fixed):
/// RuboCop aliases `on_yield` to `on_send`. Added `visit_yield_node` with
/// `check_require_parentheses_yield` and `check_omit_parentheses_yield`.
///
/// Fix 2 — Rescue/ensure scope propagation (~12k FN fixed):
/// `visit_begin_node` incorrectly propagated macro scope into rescue/ensure
/// bodies. RuboCop's `in_macro_scope?` does NOT list `rescue`/`ensure` as
/// wrappers. Fixed by manually visiting BeginNode children with `MacroScope::NotMacroScope`
/// when rescue/ensure is present.
///
/// Fix 3 — Case/when/while/until/for scope (~12k FN fixed):
/// These nodes are not wrappers in `in_macro_scope?` but nitrocop let
/// `ClassLike` scope leak through. Added scope-breaking visitors.
///
/// Fix 4 — Non-wrapper parent detection (~7k FN fixed):
/// RuboCop's `in_macro_scope?` checks the DIRECT parent node type. Calls
/// nested inside another call's arguments, assignments, arrays, etc. are NOT
/// in macro scope even if the surrounding block/class is. Implemented via
/// `scope_parent_baseline` tracking: each scope push records the parent_stack
/// depth, and `nested_in_non_wrapper()` checks if parent_stack grew since.
/// Also fixed block visitation: blocks don't push `ParentKind::Call` since in
/// Parser AST blocks WRAP the send (the block is the parent, not the send).
///
/// ## Corpus investigation (2026-03-31)
///
/// FN root cause: ordinary call-attached blocks inherited macro scope too
/// aggressively. In Parser AST the `block` node takes the surrounding
/// expression's parent, not the send as its parent, so the block body should
/// only stay in macro scope when the whole block expression is itself in macro
/// scope. nitrocop treated `Trip.new(...) { require "pry" }`,
/// `3.times.map { create ... }`, and `expect { raise subject }.to ...` as
/// macro scope because `visit_block_node` only looked at the surrounding scope.
/// Fixed by deriving the child scope for call-attached blocks from the
/// enclosing call's `nested_in_non_wrapper()` state.
///
/// A smaller FP/FN follow-up: ternary branches in class/module bodies are
/// still wrapper context for macros, but ternaries used as the predicate of an
/// outer `if`/`unless` are NOT. Model ternary branches and ternary predicates
/// separately so class-body DSL calls like `before_action` stay ignored, while
/// predicate calls like `yes_wizard? "..."` remain offenses. Also skip the
/// committed `.coverage` dotfile basename to match RuboCop's repo-target
/// selection for count-only corpus runs.
///
/// ## Corpus investigation (2026-04-01)
///
/// FN root cause: `visit_lambda_node` used `wrapper_child_scope()`, which
/// preserves macro scope through lambdas unconditionally.  But in Parser
/// AST, `-> { ... }` is `(block (send nil :lambda) ...)`, and RuboCop's
/// `in_macro_scope?` does NOT treat non-class-constructor blocks as
/// wrappers.  This meant receiverless calls inside lambdas passed as
/// arguments (e.g. `scope :x, -> { where active: true }`) were
/// incorrectly treated as macros and skipped.  Fixed by switching to
/// `call_block_child_scope()`, which checks `nested_in_non_wrapper()`
/// so that lambdas under call-argument parents break macro scope, while
/// lambdas inside wrapper blocks (`subject { -> { get :idx } }`) still
/// inherit it.  Resolved ~1k FN with 0 regressions.
///
/// ## Corpus investigation (2026-04-01, attempt 2)
///
/// FN root cause 1: block-argument-only calls (`foo &block`) were missed
/// because Prism stores `&block` in the CallNode's `block` field, not in
/// `arguments`. The check `call.arguments().is_none()` returned early.
/// Fixed by also checking for `BlockArgumentNode` in the block field.
/// Resolved ~30% of sampled FN.
///
/// FN root cause 2: `RescueModifierNode` (`foo rescue bar`) did not break
/// macro scope. In Parser AST, inline rescue wraps the call in a `rescue`
/// node, which is NOT a wrapper in RuboCop's `in_macro_scope?`. Added
/// `visit_rescue_modifier_node` that pushes `MacroScope::NotMacroScope` so receiverless
/// calls inside rescue modifiers are no longer treated as macros.
///
/// Combined: 106 FN resolved across 15 sampled repos, 0 regressions.
///
/// Remaining FN: likely from additional non-wrapper node types not yet
/// tracked on parent_stack, or subtle differences in how Prism vs Parser
/// represent certain AST structures.
///
/// ## Corpus investigation (2026-04-01, attempt 3)
///
/// FN root cause 1: `MultiWriteNode` (`a, b = call do ... end`) was not
/// treated like assignment when deciding whether a call-attached block stays
/// in macro scope. Prism uses `MultiWriteNode` for parallel assignment, so
/// `call_block_child_scope()` missed receiverless calls such as
/// `planned? sub` / `call_event "x", event` inside those blocks. Fixed by
/// pushing `ParentKind::Assignment` while visiting the RHS of MultiWriteNode.
///
/// FN root cause 2: flow-control nodes like `NextNode` were not tracked on
/// `parent_stack`. In Parser AST, `next send_file static_file` gives the call
/// a direct non-wrapper parent, so macro scope must break there. Fixed by
/// tracking `return`/`break`/`next` arguments as `ParentKind::FlowControl`.
///
/// ## Corpus investigation (2026-04-01, attempt 4)
///
/// FN root cause 1 (~130 FN): `InterpolatedStringNode` / `InterpolatedSymbolNode`
/// (Parser's `dstr`/`dsym`) are NOT wrappers in `in_macro_scope?`, but
/// nitrocop did not track them as non-wrapper parents. Calls inside `#{}`
/// string interpolation in macro scope were incorrectly treated as macros.
/// Fixed by pushing `ParentKind::Interpolation` when visiting interpolated
/// string/symbol nodes. This resolved tdiary (67 FN), aruba (9 FN), and
/// many others.
///
/// FN root cause 2 (~19 FN): `PreExecutionNode` (`BEGIN { }`) was not
/// handled. In Parser AST, `preexe` is NOT a wrapper in `in_macro_scope?`.
/// Added `visit_pre_execution_node` pushing `MacroScope::NotMacroScope`. Also added
/// `visit_post_execution_node` for `END { }` symmetry.
///
/// FN root cause 3 (~4 FN): `CaseMatchNode` (`case...in` pattern matching)
/// was not handled, unlike `CaseNode` (`case...when`). Neither is a wrapper
/// in `in_macro_scope?`. Added `visit_case_match_node` pushing `MacroScope::NotMacroScope`.
///
/// FN root cause 4 (~30+ FN): Operator assignment nodes (`+=`, `-=`, `||=`,
/// `&&=`, etc.) were not tracked as `ParentKind::Assignment`. Added visitors
/// for all `*OperatorWriteNode`, `*OrWriteNode`, `*AndWriteNode` variants
/// plus `Call*WriteNode` and `Index*WriteNode`.
///
/// Combined: 289 FN resolved across 15 sampled repos, 0 regressions.
///
/// ## Corpus investigation (2026-04-01, attempt 5)
///
/// FN root cause 1: pure `BeginNode`s (`x = begin ... end`, `lhs || begin ... end`)
/// preserved macro scope unconditionally. RuboCop only treats `kwbegin` as a
/// wrapper when the whole begin expression is already in macro scope; an outer
/// assignment/logical-op parent still breaks it. Fixed by deriving pure-begin
/// child scope from `nested_in_non_wrapper()`, matching `if`/`unless`.
///
/// FN root cause 2: `InterpolatedXStringNode` (`%x{#{...}}`, common in Opal)
/// was not tracked as an interpolation parent. Receiverless calls inside the
/// embedded `#{...}` were therefore treated like top-level/class-body macros.
/// Added interpolation-parent tracking for interpolated x-strings and
/// interpolated regular expressions.
///
/// ## Variant fix: omit_parentheses style FP from when clause handling
///
/// FP root cause: `visit_when_node` pushed `ParentKind::When` before visiting
/// conditions, then popped it before visiting statements. This meant calls in the
/// when body (e.g., `comments_path(...)` inside `when :comment`) had their
/// parent_stack topped with whatever context preceded the when node, not `When`.
/// RuboCop's `node.parent.when_type?` correctly returns true for calls inside
/// when bodies. Fixed by re-pushing `ParentKind::When` before visiting statements,
/// then popping after. This reduces FP by ~17 in the e621ng repo alone.
///
/// ## Prior validation (2026-04-01, attempt 5)
///
/// Validation: `python3 scripts/check_cop.py Style/MethodCallWithArgsParentheses
/// --rerun --clone --sample 15` reported `0` new FP, `0` new FN, and all `41`
/// sampled oracle FN resolved.
///
/// ## Variant fix (2026-04-08)
///
/// `omit_parentheses` still diverged in Prism-specific grouping edge cases:
///
/// 1. Parenthesized ambiguous descendants such as `Array(foo((bar || [])))`
///    were flagged because ambiguity checks stopped at `ParenthesesNode`.
///    RuboCop still sees the inner grouped `||` descendant and allows the
///    outer parens.
///
/// 2. Inner calls wrapped in grouping parentheses such as `foo((bar(1)))`
///    and `if (server = response.take(1))` were skipped because Prism keeps
///    the surrounding parent call/assignment on the stack unless
///    `ParenthesesNode` is tracked explicitly. RuboCop sees an intervening
///    `begin` parent, so grouped inner calls are not treated as direct
///    arguments or direct assignment-in-condition children.
///
/// 3. Assigned values under assignment-like parent sends such as
///    `session[:x] = foo(bar)` and `self.value = foo(bar)` were skipped because
///    Prism models `[]=` and setters as `CallNode`s. RuboCop's
///    `call_as_argument_or_chain?` only exempts children that appear before the
///    parent's assignment operator; the RHS call after `=` remains an offense.
///    Fixed by treating call children that start after `equal_loc` as
///    `ParentKind::Assignment`, while leaving receiver/index children before `=`
///    in ordinary call context.
///
/// Hash-literal allowances follow RuboCop's broader descendant scan: once any
/// direct argument is a send, a braced hash descendant anywhere under the outer
/// call keeps the outer parentheses. Grouped direct arguments still do not
/// satisfy the direct-send gate.
///
/// ## Variant fix (2026-04-10)
///
/// Two remaining `omit_parentheses` mismatches came from Prism context that
/// only shows up under explicit variant-config tests:
///
/// 1. Single-statement `if`/`unless`/`when` bodies were missing the direct
///    parent node RuboCop sees in Parser AST. RuboCop allows parentheses for
///    assignment RHS calls when the assignment expression is the value of a
///    single-statement conditional/when branch, such as `@x = Foo.new(...)`
///    under `if`/`else` and modifier conditionals. Model that parent only for
///    single-statement branch bodies; multi-statement bodies still behave like
///    Parser's synthetic `begin`.
///
/// 2. Hash-value-omission calls (`foo(value:)`) were treated as always
///    exempt. RuboCop only keeps parentheses when the call is in a
///    conditional-style parent or is not the last expression. Added explicit
///    non-last-expression tracking and narrowed the exemption to those cases.
///
/// ## Variant fix (2026-04-16)
///
/// `omit_parentheses` still diverged in two Parser-vs-Prism contexts:
///
/// 1. RuboCop's ambiguous-descendant check scans the whole send node, not just
///    its direct arguments. Parenthesized calls whose RECEIVER chain contains a
///    logical operator or a block (for example
///    `(ignored_organisations || []).join(", ")` and
///    `%w[1 2 3].map { ... }.join("-")`) must keep parentheses.
///
/// 2. When a chained call takes a block expression as its receiver
///    (`expect do ... end.to ...`), Parser gives expressions inside that block
///    a direct `block` parent, not the outer chained `send`. Prism traversal
///    was leaking the outer `Call` parent into the block body, so inner calls
///    like `Class.new(TestInteraction) do ... end` were incorrectly treated as
///    chained arguments and skipped.
///
/// ## Failed attempt (2026-04-16, PR #2197, closed)
///
/// An agent tried to fix the remaining `omit_parentheses` divergence by:
/// - Adding `ParentKind::CallAssignedRhs` and `ParentKind::Super` variants
/// - Adding a `block_parent_is_call_like: Vec<bool>` stack to track whether
///   the ancestor through a block is a call-like expression
/// - Introducing `visit_parser_block_body` that pops/restores parents to
///   mimic Parser's `block` wrapper semantics
/// - Changing `call_in_argument_with_block` to consult
///   `current_parent_is_call_like()` / `in_call_like_block_parent()`
/// - Visiting `SuperNode` arguments for nested omit-paren calls
///
/// Result: -2,256 FP, +2,897 FN (net +641 worse).  The block-parent tracking
/// was too aggressive — it suppressed legitimate omit-paren offenses that
/// live inside blocks whose parent happens to be a call-like node.  The
/// specific pattern that over-suppresses: `foo.bar { |x| baz(x) }` where
/// `baz(x)` should still be an omit offense but the new logic marks it as
/// an allowed call-in-block-in-call.
///
/// What a correct fix would need: `call_in_argument_with_block` should only
/// allow parens when the block is being passed AS AN ARGUMENT to the outer
/// call (i.e., the outer call has a trailing block literal), not whenever
/// the outer context happens to be call-like.  Verify with RuboCop on
/// `AlchemyCMS/alchemy_cms/app/components/alchemy/admin/tags_list.rb:26`
/// (FN) and a simple `foo.bar { baz(x) }` (must flag baz) before changing
/// parent tracking.
///
/// ## Variant fix (2026-04-17)
///
/// The remaining `omit_parentheses` false positives came from two narrow
/// Parser-vs-Prism parent mismatches:
///
/// 1. Nested calls inside `super(...)` arguments were still treated as plain
///    top-level calls because Prism's `SuperNode` children were not visited
///    with any omit-parentheses parent context. RuboCop sees the direct parent
///    as `super`, which makes calls like `errors.local_attribute(attribute)` a
///    legitimate parenthesized argument.
///
/// 2. `call_in_argument_with_block?` depends on a DIRECT `block` parent whose
///    own parent is call-like. Prism traversal was only tracking the enclosing
///    call parent, so it still flagged parenthesized calls in single-statement
///    block bodies such as `run_callbacks(:execute) do execute end` when that
///    block expression was used as a setter RHS or chained receiver. Fixed by
///    modeling only the direct single-statement block body parent, which keeps
///    `foo.bar { baz(x) }` as an offense while allowing `foo.bar { baz(x) }.qux`.
///
/// ## Variant fix (2026-04-19)
///
/// `has_hash_value_omission` / `has_keyword_hash_value_omission` were
/// returning true when ANY pair had value omission. RuboCop's rule is
/// `last_argument.pairs.last&.value_omission?` — only the LAST pair
/// matters. Calls like
/// `FactoryBot.create(:project, inactive:, main_language: language)` must
/// still fire when the last pair is a regular `k: v` even though an
/// earlier pair uses the shorthand.
///
/// ## Variant fix (2026-04-19, unary descendants)
///
/// RuboCop treats unary operator descendants as ambiguous in
/// `omit_parentheses`, so outer-call parens are allowed for cases like
/// `some_helper(..., readonly: !editable?)`,
/// `fold!(current_user.id, !was_folded)`, and
/// `update(folded: !@node.folded)`. Prism models those as unary call nodes,
/// but nitrocop only handled signed numerics and `+@`/`-@`, so it still
/// flagged the outer call. Match RuboCop by treating any Prism unary
/// operation descendant as ambiguous.
///
/// ## Variant fix (2026-04-20)
///
/// Two remaining `omit_parentheses` mismatches came from variant-run context:
///
/// 1. Outer calls with lambda literals in their receiver/argument tree, such
///    as `{ tags_formatter: ->(tags) { ... } }.merge(options)`, were still
///    flagged. RuboCop's descendant scan sees the lambda's underlying `block`
///    node (`any_block`) and treats the outer call as ambiguous, so the
///    parentheses stay.
///
/// 2. Variant corpus runs use an explicit temporary `--config` file. RuboCop
///    does NOT apply nested `.rubocop.yml` overrides when a config path is
///    passed explicitly, but nitrocop was still sweeping subdirectories under
///    the config file's parent. In CI this temp directory can also contain the
///    cloned repos, so nested repo overrides leaked into the style-override
///    run and caused large omit-parentheses FP/FN swings. Matching RuboCop
///    requires disabling directory overrides for explicit `--config` loads.
///
/// ## Variant fix (2026-04-25, oracle: 1,997 FP / 15,958 FN)
///
/// Three independent issues were leaking through the omit_parentheses variant
/// even after the previous fixes:
///
/// 1. **Block-body parent leakage.** When a block body contained an
///    assignment, `visit_*_write_node` pushed `Assignment` and the *outer*
///    expression's parent (e.g., `ConditionalBody`, another `Assignment`)
///    leaked through as the assignment's grandparent. `assignment_in_condition`
///    then incorrectly returned true for calls like `name = foo(...)` and
///    `value = foo(...)` inside `.map do ... end` blocks. Similarly, the
///    `CallLikeBlockBody` marker pushed for an outer call's block body
///    propagated into a *nested* call's block body, so `link_to(...)` inside
///    `content_tag(...) do ... end` inside `sorted_tags.map do ... end.join`
///    was treated as the outer block's argument context. Fixed by pushing a
///    new `ParentKind::Block` boundary marker at the entry of every
///    block/lambda body. `assignment_in_condition?` now sees `Block` as the
///    grandparent and returns false; `effective_parent()` skips the marker
///    for `call_in_literals?` / `call_in_logical_operators?` so the
///    block-pair-or-array-or-logical-op shape is preserved.
///
/// 2. **Lambda-as-call-argument body.** `visit_lambda_node` did not pop the
///    enclosing `Call`/`CallAssignedRhs` parent or push `CallLikeBlockBody`,
///    so calls inside `options[:converter] = ->(x) { ObjectThing.converter(x) }`
///    were flagged even though RuboCop sees the lambda's parent (in Parser
///    AST: a `block` whose own parent is the `[]=` send) as a call and exempts
///    inner calls via `call_in_argument_with_block?`. Aligned `visit_lambda_node`
///    with `visit_block_node`'s call-like-parent handling.
///
/// 3. **Ambiguous descendants behind ranges and interpolation.** RuboCop uses
///    `node.descendants.any?` over the full subtree, so signed numerics inside
///    range bounds (`vectors[1..-1]`, `Concurrent::Array.new(names[0...-1])`)
///    and ternaries inside `#{...}` interpolation (`order("name #{cond ? "DESC" : "ASC"}")`)
///    keep outer parentheses. nitrocop's hand-rolled descendant recursion
///    only walked specific compound types (call args/recv, array elements,
///    hash pairs), missing range bounds and interpolation parts. Replaced
///    with a Prism `Visit`-based traversal that visits every node in the
///    subtree and runs the per-node ambiguity predicate, matching RuboCop's
///    `descendants.any?` semantics exactly.
///
/// ## Variant fix (2026-04-25, sibling-depth + lambda case/in + rescue main body)
///
/// Three narrower omit_parentheses divergences remained:
///
/// 1. `require_parentheses_for_hash_value_omission?` treated ANY outer
///    non-last statement as meaning the current call was non-value-returning.
///    RuboCop only checks the call itself (or its direct assignment parent)
///    for right siblings. That meant shorthand keyword calls inside a method
///    body were incorrectly exempted whenever the enclosing `def` had later
///    sibling methods. Reset `non_last_expression_depth` at `def` body entry so
///    outer class/module siblings no longer leak into the method body.
///
/// 2. Calls nested under `case/in` branches inside a lambda literal that is
///    passed as a call argument were incorrectly treated like direct lambda-body
///    calls and inherited `CallLikeBlockBody`. In Parser AST those calls have an
///    `in_pattern` direct parent, so the parentheses must still be omitted.
///    Added `visit_in_node` parent tracking so `case/in` bodies break the
///    direct-block allowance.
///
/// 3. RuboCop allows shorthand keyword calls in the MAIN expression of a
///    `begin ... rescue` because the rescue clause makes that expression
///    non-value-returning. Prism did not model that direct parent, so nitrocop
///    still flagged calls like `execute(build_cloud:, user:)` before `rescue`.
///    Added a dedicated rescue-main parent context for the single-statement
///    case, matching the observed corpus pattern without broadening nested
///    descendants.
///
/// ## Variant fix (2026-04-25, oracle: 643 FP / 8,662 FN)
///
/// Four narrower omit_parentheses divergences remained after the previous
/// rounds:
///
/// 1. **`yield(...)` skipped `assignment_in_condition?`.** RuboCop aliases
///    `on_yield` to `on_send` and runs the same `legitimate_call_with_parentheses?`
///    chain, so `lhs = yield(...) if cond` and
///    `lhs, rhs = yield(...) unless cond` keep their parens. nitrocop's
///    `check_omit_parentheses_yield` only consulted a subset of the guards and
///    flagged these. Added `assignment_in_condition()` (and `WhenBody` parents
///    that mirror `parent.when_type?`) to the yield guard list.
///
/// 2. **Single-line parent for hash-value-omission.** RuboCop's
///    `require_parentheses_for_hash_value_omission?` returns true when
///    `node.parent&.single_line?`, which keeps parens on calls like
///    `@x ||= User.find_by(api_key:)`. nitrocop only checked for conditional
///    parents, so single-line assignments slipped through. Added a parallel
///    `parent_loc_stack` that records the source span of each pushed parent
///    AST node, plus `parent_is_single_line()` consulted from the hash-value
///    omission rule. Write-node visitors and `visit_call_node` now push their
///    locations alongside `Assignment`/`Call`/`CallAssignedRhs`.
///
/// 3. **Outer Call leaked through `begin ... end`.** `kwbegin` is not a
///    `call_as_argument_or_chain?` parent in RuboCop, but nitrocop's
///    `visit_begin_node` left the surrounding `Call`/`CallAssignedRhs`/`Super`
///    on `parent_stack`. Calls inside `||= begin ... arr.join("_") end.to_sym`
///    therefore saw the chained `.to_sym`'s `Call` parent and were exempted
///    incorrectly. Pop and restore the outer call-like parent at begin entry,
///    mirroring `visit_block_node`.
///
/// 4. **Outer Call leaked through `case ... else <call> end.chain`.** Same
///    pattern as begin: the `else` branch's call has the `case` node as its
///    Parser parent, not the chained call. nitrocop let the outer `Call`
///    leak through `visit_case_node` (and `visit_case_match_node`) into the
///    else statements, so `else resize_to_limit_options(width, height) end
///    .transform_values!` was treated as a chain argument. Pop and restore
///    the outer call-like parent in both case visitors.
///
/// ## Variant fix (2026-04-26, oracle: 458 FP / 8,423 FN + default 0 FP / 2 FN)
///
/// Four narrower divergences remained on the latest oracle:
///
/// 1. **Default-config FN regression in `visit_begin_node`.** The previous
///    fix that pops an outer `Call`/`CallAssignedRhs`/`Super` parent at begin
///    entry also caused `nested_in_non_wrapper()` to read a parent_stack with
///    the leaked parent already gone. For
///    `Foo.config.x = begin ...; raise "msg"; ...; end`, that meant the
///    surrounding setter's `Call` parent vanished before the wrapper-vs-non-
///    wrapper decision, so the begin's child scope inherited macro scope from
///    top level and `raise "msg"`/`puts "..."` were silently treated as
///    macros. Fixed by capturing `nested_in_non_wrapper()` BEFORE popping
///    `leaked_parent`.
///
/// 2. **`assignment_in_condition?` missed ternary branches.** RuboCop's
///    `grandparent.conditional?` is true for ternary `if` nodes, so
///    `value = ref.is_tagged_with?(...)` inside `cond ? ... : value = ...`
///    keeps its parens. nitrocop only matched `Conditional`/`ConditionalBody`/
///    `When`/`WhenBody` as the grandparent. Added `TernaryBranch` so ternary
///    `:` branches with assignment RHSs no longer false-positive.
///
/// 3. **Multi-statement `when`/`if`/`else` bodies leaked outer
///    `ConditionalBody`/`WhenBody` as the assignment grandparent.** Parser AST
///    wraps multi-statement bodies in a synthetic `:begin`. RuboCop's
///    `assignment_in_condition?` then sees the begin (not a conditional/when)
///    as grandparent and returns false. nitrocop's
///    `visit_statements_with_parent` only pushed the parent kind for SINGLE-
///    statement bodies, leaving multi-statement bodies to inherit whatever
///    parent the surrounding `if`/`case` carried. So
///    `case x; when :a; lvasgn1; lvasgn2 = call(...); ...; end` inside an
///    enclosing `if cond ... end` falsely matched
///    `Assignment + ConditionalBody` and skipped the offense. Added a
///    `ParentKind::Begin` boundary marker pushed for multi-statement bodies
///    in both `visit_statements_node` and `visit_statements_with_parent`
///    (and added it to `nested_in_non_wrapper`'s wrapper exempt list because
///    `:begin`/`:kwbegin` ARE wrappers in `in_macro_scope?`).
///
/// 4. **`non_last_expression_depth` leaked through block bodies.** RuboCop's
///    `last_expression?` only inspects the call's immediate right sibling
///    (or the right sibling of its assignment parent). nitrocop tracked a
///    depth counter that only reset at `def` body entry, so an outer
///    non-last `if` statement above a `.map do ... end` block kept the
///    depth at `1` for value-omission calls inside the block. Reset
///    `non_last_expression_depth` at every block/lambda body entry so each
///    block body computes value-omission siblings independently.
///
/// ## Variant fix follow-up (2026-04-26): record block source span
///
/// The `non_last_expression_depth` reset above introduced ~800 new FP on
/// hash-value-omission calls inside SINGLE-LINE blocks
/// (`let(:x) { create(:y, foo:) }`, `.map { |x| Foo.new(x:) }`). Before the
/// reset, the leaked outer depth happened to make
/// `require_parentheses_for_hash_value_omission?` return true via the
/// `!last_expression?` branch — incidentally agreeing with RuboCop, which
/// allows the parens via `node.parent&.single_line?` (the block is
/// single-line). With the depth reset, that incidental allowance went away.
///
/// Implement the actual `parent.single_line?` semantics: push every
/// `ParentKind::Block` / `ParentKind::CallLikeBlockBody` WITH the block
/// node's source span. `parent_is_single_line()` then returns true for
/// braced single-line block parents and false for `do ... end` multi-line
/// blocks, matching RuboCop. Threaded through `visit_node_with_parent`
/// and `visit_statements_with_parent` via `_loc` variants that accept an
/// `Option<(usize, usize)>` location for the synthetic `CallLikeBlockBody`
/// boundary.
pub struct MethodCallWithArgsParentheses;

/// Check if a method name matches any pattern in the list (regex-style).
fn matches_any_pattern(name_str: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if re.is_match(name_str) {
                return true;
            }
        }
    }
    false
}

/// Check if the method name starts with an uppercase letter (CamelCase).
fn is_camel_case_method(name: &[u8]) -> bool {
    name.first().is_some_and(|b| b.is_ascii_uppercase())
}

/// Check if a CallNode is a class constructor pattern:
/// `Class.new`, `Module.new`, `Struct.new`, or `Data.define`.
/// This matches RuboCop's `class_constructor?` node pattern.
fn is_class_constructor(call: &ruby_prism::CallNode<'_>) -> bool {
    let method_name = call.name().as_slice();
    let recv = match call.receiver() {
        Some(r) => r,
        None => return false,
    };

    // Check for `Class.new`, `Module.new`, `Struct.new`
    if method_name == b"new" {
        if let Some(cr) = recv.as_constant_read_node() {
            let cname = cr.name().as_slice();
            return cname == b"Class" || cname == b"Module" || cname == b"Struct";
        }
        // Also handle fully qualified ::Class.new etc.
        if let Some(cp) = recv.as_constant_path_node() {
            if cp.parent().is_none() {
                if let Some(child_name) = cp.name() {
                    let cname = child_name.as_slice();
                    return cname == b"Class" || cname == b"Module" || cname == b"Struct";
                }
            }
        }
    }

    // Check for `Data.define`
    if method_name == b"define" {
        if let Some(cr) = recv.as_constant_read_node() {
            return cr.name().as_slice() == b"Data";
        }
        if let Some(cp) = recv.as_constant_path_node() {
            if cp.parent().is_none() {
                if let Some(child_name) = cp.name() {
                    return child_name.as_slice() == b"Data";
                }
            }
        }
    }

    false
}

// Macro scope tracking uses shared MacroScope from access_modifier_predicates.

/// Parent node type for omit_parentheses context checks.
#[derive(Clone, Copy, PartialEq)]
enum ParentKind {
    Array,
    Pair,
    Range,
    Splat,
    KwSplat,
    BlockPass,
    TernaryBranch,
    TernaryPredicate,
    LogicalOp,
    Call,
    OptArg,
    KwOptArg,
    ClassSingleLine,
    When,
    WhenBody,
    InPatternBody,
    MatchPattern,
    Super,
    CallAssignedRhs,
    Assignment,
    Conditional,
    ConditionalBody,
    ClassConstructor,
    CallLikeBlockBody,
    ConstantPath,
    Grouped,
    FlowControl,
    Interpolation,
    RescueMainBody,
    /// Boundary marker pushed at the entry of every block/lambda body. Prevents
    /// outer parents (Assignment, ConditionalBody, etc.) from leaking into the
    /// block body's statements. RuboCop's parent walks stop at the block node,
    /// so callers nested inside the block must not see the surrounding
    /// expression's parent as their grandparent.
    Block,
    /// Synthetic begin marker pushed for multi-statement bodies (def/block/
    /// when/if/else/etc.). Parser AST wraps multi-statement bodies as a
    /// `:begin` node. RuboCop's `assignment_in_condition?` therefore sees a
    /// begin (not the surrounding conditional/when) as the grandparent of an
    /// inner assignment. This marker reproduces that boundary so outer
    /// `ConditionalBody`/`WhenBody`/etc. don't leak across multi-statement
    /// lists. `:begin`/`:kwbegin` ARE wrappers in `in_macro_scope?`, so this
    /// kind is included in `nested_in_non_wrapper`'s exempt list.
    Begin,
}

impl Cop for MethodCallWithArgsParentheses {
    fn name(&self) -> &'static str {
        "Style/MethodCallWithArgsParentheses"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        if source.path.file_name().and_then(|name| name.to_str()) == Some(".coverage") {
            return;
        }

        let enforced_style = config.get_str("EnforcedStyle", "require_parentheses");
        let ignore_macros = config.get_bool("IgnoreMacros", true);
        let allowed_methods = config.get_string_array("AllowedMethods");
        let allowed_patterns = config.get_string_array("AllowedPatterns");
        let included_macros = config.get_string_array("IncludedMacros");
        let included_macro_patterns = config.get_string_array("IncludedMacroPatterns");
        let allow_multiline = config.get_bool("AllowParenthesesInMultilineCall", false);
        let allow_chaining = config.get_bool("AllowParenthesesInChaining", false);
        let allow_camel = config.get_bool("AllowParenthesesInCamelCaseMethod", false);
        let allow_interp = config.get_bool("AllowParenthesesInStringInterpolation", false);

        let mut visitor = ParenVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            enforced_style,
            ignore_macros,
            allowed_methods: allowed_methods.as_deref(),
            allowed_patterns: allowed_patterns.as_deref(),
            included_macros: included_macros.as_deref(),
            included_macro_patterns: included_macro_patterns.as_deref(),
            allow_multiline,
            allow_chaining,
            allow_camel,
            allow_interp,
            scope_stack: vec![],
            scope_parent_baseline: vec![0],
            parent_stack: vec![],
            parent_loc_stack: vec![],
            in_interpolation: false,
            in_endless_def: false,
            non_last_expression_depth: 0,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct ParenVisitor<'a> {
    cop: &'a MethodCallWithArgsParentheses,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    enforced_style: &'a str,
    ignore_macros: bool,
    allowed_methods: Option<&'a [String]>,
    allowed_patterns: Option<&'a [String]>,
    included_macros: Option<&'a [String]>,
    included_macro_patterns: Option<&'a [String]>,
    allow_multiline: bool,
    allow_chaining: bool,
    allow_camel: bool,
    allow_interp: bool,
    scope_stack: Vec<MacroScope>,
    /// Records parent_stack.len() at each scope push, so we can tell whether
    /// a parent_stack entry belongs to the CURRENT scope or an outer one.
    scope_parent_baseline: Vec<usize>,
    parent_stack: Vec<ParentKind>,
    /// Parallel to `parent_stack`. `Some((start, end))` records the source
    /// span of the AST node that pushed the corresponding `ParentKind`, which
    /// powers RuboCop's `node.parent&.single_line?` check
    /// (`require_parentheses_for_hash_value_omission?`). `None` is used for
    /// synthetic boundary markers (e.g. `Block`) that don't correspond to a
    /// concrete node.
    parent_loc_stack: Vec<Option<(usize, usize)>>,
    in_interpolation: bool,
    in_endless_def: bool,
    non_last_expression_depth: usize,
}

impl ParenVisitor<'_> {
    fn push_macro_scope(&mut self, scope: MacroScope) {
        self.scope_stack.push(scope);
        self.scope_parent_baseline.push(self.parent_stack.len());
    }

    fn pop_scope(&mut self) {
        self.scope_stack.pop();
        self.scope_parent_baseline.pop();
    }

    /// Push a parent kind without an associated source span. Use for synthetic
    /// boundary markers (Block, ConditionalBody, WhenBody) and for parents
    /// where the location isn't readily available.
    fn push_parent(&mut self, kind: ParentKind) {
        self.parent_stack.push(kind);
        self.parent_loc_stack.push(None);
    }

    /// Push a parent kind together with the source span of the AST node it
    /// represents. The span is used by `parent_is_single_line()` to mirror
    /// RuboCop's `node.parent&.single_line?`.
    fn push_parent_with_loc(&mut self, kind: ParentKind, loc: ruby_prism::Location<'_>) {
        self.parent_stack.push(kind);
        self.parent_loc_stack
            .push(Some((loc.start_offset(), loc.end_offset())));
    }

    fn pop_parent(&mut self) -> Option<ParentKind> {
        self.parent_loc_stack.pop();
        self.parent_stack.pop()
    }

    fn immediate_parent(&self) -> Option<ParentKind> {
        self.parent_stack.last().copied()
    }

    /// Mirrors RuboCop's `node.parent&.single_line?`. Returns true when the
    /// immediate parent has a recorded source span and that span is on a
    /// single line.
    fn parent_is_single_line(&self) -> bool {
        let Some((start, end)) = self.parent_loc_stack.last().copied().flatten() else {
            return false;
        };
        let (start_line, _) = self.source.offset_to_line_col(start);
        let (end_line, _) = self.source.offset_to_line_col(end);
        start_line == end_line
    }

    /// Mirrors RuboCop's
    /// `parent = node.parent&.any_block_type? ? node.parent.parent : node.parent`.
    /// Several `omit_parentheses` checks (`call_in_literals?`,
    /// `call_in_logical_operators?`) look "through" a block parent to the
    /// expression that actually wraps it. Since `Block` is a synthetic
    /// boundary marker we always push at block/lambda body entry, skip it so
    /// the check sees the enclosing Pair/Array/LogicalOp/etc.
    fn effective_parent(&self) -> Option<ParentKind> {
        let mut idx = self.parent_stack.len();
        if idx == 0 {
            return None;
        }
        idx -= 1;
        if matches!(self.parent_stack[idx], ParentKind::Block) {
            if idx == 0 {
                return None;
            }
            idx -= 1;
        }
        Some(self.parent_stack[idx])
    }

    fn direct_call_like_parent(&self) -> bool {
        matches!(
            self.immediate_parent(),
            Some(ParentKind::Call | ParentKind::ClassConstructor | ParentKind::Super)
        )
    }

    fn block_parent_is_call_like(&self) -> bool {
        matches!(
            self.immediate_parent(),
            Some(
                ParentKind::Call
                    | ParentKind::ClassConstructor
                    | ParentKind::Super
                    | ParentKind::CallAssignedRhs
            )
        )
    }

    fn in_non_last_expression(&self) -> bool {
        self.non_last_expression_depth > 0
    }

    fn is_macro_scope(&self) -> bool {
        crate::cop::shared::access_modifier_predicates::in_macro_scope(&self.scope_stack)
    }

    /// Check if the call is nested inside a non-wrapper parent within the
    /// current scope. RuboCop's `in_macro_scope?` checks the DIRECT parent
    /// node type — only wrappers (begin, block, if) and class-like nodes
    /// propagate macro scope. Any other parent (send, assignment, array, etc.)
    /// breaks it. We detect this by checking whether parent_stack has grown
    /// since the current scope was entered.
    fn nested_in_non_wrapper(&self) -> bool {
        let baseline = self.scope_parent_baseline.last().copied().unwrap_or(0);
        self.parent_stack[baseline..].iter().any(|kind| {
            !matches!(
                kind,
                ParentKind::TernaryBranch
                    | ParentKind::ClassConstructor
                    | ParentKind::CallLikeBlockBody
                    | ParentKind::Grouped
                    | ParentKind::ConditionalBody
                    | ParentKind::WhenBody
                    | ParentKind::Block
                    | ParentKind::Begin
            )
        })
    }

    fn visit_statements_with_parent<'pr>(
        &mut self,
        node: &ruby_prism::StatementsNode<'pr>,
        parent_kind: ParentKind,
    ) {
        self.visit_statements_with_parent_loc(node, parent_kind, None);
    }

    fn visit_statements_with_parent_loc<'pr>(
        &mut self,
        node: &ruby_prism::StatementsNode<'pr>,
        parent_kind: ParentKind,
        parent_loc: Option<(usize, usize)>,
    ) {
        let body = node.body();
        let single_statement = body.len() == 1;
        let can_inherit_direct_parent = single_statement
            && body
                .iter()
                .next()
                .is_some_and(|stmt| stmt.as_begin_node().is_none());

        // Multi-statement bodies wrap as `:begin` in Parser AST. Push the
        // synthetic boundary so descendants don't see the surrounding
        // ConditionalBody/WhenBody as their grandparent in
        // `assignment_in_condition`. Single-statement bodies (or a single
        // begin child) keep the existing behavior.
        let pushed_begin = !can_inherit_direct_parent && body.len() > 1;
        if pushed_begin {
            self.push_parent(ParentKind::Begin);
        }

        for (index, stmt) in body.iter().enumerate() {
            let is_not_last = index + 1 < body.len();
            if is_not_last {
                self.non_last_expression_depth += 1;
            }

            if can_inherit_direct_parent {
                if let Some((start, end)) = parent_loc {
                    self.parent_stack.push(parent_kind);
                    self.parent_loc_stack.push(Some((start, end)));
                } else {
                    self.push_parent(parent_kind);
                }
            }
            self.visit(&stmt);
            if can_inherit_direct_parent {
                self.pop_parent();
            }

            if is_not_last {
                self.non_last_expression_depth -= 1;
            }
        }

        if pushed_begin {
            self.pop_parent();
        }
    }

    fn visit_node_with_parent_loc<'pr>(
        &mut self,
        node: &ruby_prism::Node<'pr>,
        parent_kind: ParentKind,
        parent_loc: Option<(usize, usize)>,
    ) {
        if let Some(stmts) = node.as_statements_node() {
            self.visit_statements_with_parent_loc(&stmts, parent_kind, parent_loc);
        } else if node.as_begin_node().is_none() {
            if let Some((start, end)) = parent_loc {
                self.parent_stack.push(parent_kind);
                self.parent_loc_stack.push(Some((start, end)));
            } else {
                self.push_parent(parent_kind);
            }
            self.visit(node);
            self.pop_parent();
        } else {
            self.visit(node);
        }
    }

    /// Derive child scope for wrapper nodes (begin, block, if branches).
    /// Inherits macro scope from parent, matching rubocop-ast's in_macro_scope?.
    fn wrapper_child_scope(&self) -> MacroScope {
        if self.is_macro_scope() {
            MacroScope::InMacroScope
        } else {
            MacroScope::NotMacroScope
        }
    }

    /// For a block attached to a regular method call, preserve macro scope only
    /// when the whole block expression is itself in macro scope. If the call is
    /// nested under assignment/chaining/arguments/etc., Parser would give the
    /// block that non-wrapper parent and macro scope must not leak into the
    /// block body.
    fn call_block_child_scope(&self) -> MacroScope {
        if self.nested_in_non_wrapper() {
            MacroScope::NotMacroScope
        } else {
            self.wrapper_child_scope()
        }
    }

    fn check_require_parentheses(&mut self, call: &ruby_prism::CallNode<'_>) {
        let name = call.name().as_slice();

        // Skip operators and setters
        if method_identifier_predicates::is_operator_method(name)
            || method_identifier_predicates::is_setter_method(name)
        {
            return;
        }

        let has_parens = call.opening_loc().is_some();
        if has_parens {
            return;
        }

        // Must have arguments (regular args or block pass like &block)
        let has_block_arg = call
            .block()
            .is_some_and(|b| b.as_block_argument_node().is_some());
        if call.arguments().is_none() && !has_block_arg {
            return;
        }

        let name_str = std::str::from_utf8(name).unwrap_or("");
        let is_receiverless = call.receiver().is_none();

        // AllowedMethods: exempt specific method names
        if let Some(methods) = self.allowed_methods {
            if methods.iter().any(|m| m == name_str) {
                return;
            }
        }

        // AllowedPatterns: exempt methods matching patterns
        if let Some(patterns) = self.allowed_patterns {
            if matches_any_pattern(name_str, patterns) {
                return;
            }
        }

        // IgnoreMacros: skip macro calls (receiverless + in macro scope)
        // unless they are in IncludedMacros or IncludedMacroPatterns.
        if is_receiverless
            && self.ignore_macros
            && self.is_macro_scope()
            && !self.nested_in_non_wrapper()
        {
            let in_included = self
                .included_macros
                .is_some_and(|macros| macros.iter().any(|m| m == name_str));
            let in_included_patterns = self
                .included_macro_patterns
                .is_some_and(|patterns| matches_any_pattern(name_str, patterns));

            if !in_included && !in_included_patterns {
                return;
            }
        }

        // RuboCop reports the offense at the start of the full expression (including
        // receiver), not at the method name. Use call.location() to match.
        let (line, column) = self
            .source
            .offset_to_line_col(call.location().start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Use parentheses for method calls with arguments.".to_string(),
        ));
    }

    fn check_omit_parentheses(&mut self, call: &ruby_prism::CallNode<'_>) {
        let name = call.name().as_slice();

        let has_parens = call.opening_loc().is_some();
        if !has_parens {
            return;
        }

        // syntax_like_method_call? — implicit call (.()) or operator methods
        if method_identifier_predicates::is_operator_method(name) {
            return;
        }

        // Check for implicit call: foo.() has call_operator_loc but no message_loc
        if call.message_loc().is_none() && call.call_operator_loc().is_some() {
            return;
        }

        // inside_endless_method_def? — parens required in endless methods
        if self.in_endless_def && call.arguments().is_some() {
            return;
        }

        // method_call_before_constant_resolution? — parent is ConstantPathNode
        if self.immediate_parent() == Some(ParentKind::ConstantPath) {
            return;
        }

        // super_call_without_arguments? — not applicable for CallNode

        // allowed_camel_case_method_call?
        if is_camel_case_method(name) && (call.arguments().is_none() || self.allow_camel) {
            return;
        }

        // AllowParenthesesInStringInterpolation
        if self.allow_interp && self.in_interpolation {
            return;
        }

        // legitimate_call_with_parentheses? — many sub-checks
        if self.legitimate_call_with_parentheses(call) {
            return;
        }

        // require_parentheses_for_hash_value_omission?
        if self.require_parentheses_for_hash_value_omission(call) {
            return;
        }

        let open_loc = match call.opening_loc() {
            Some(loc) => loc,
            None => return,
        };
        let (line, column) = self.source.offset_to_line_col(open_loc.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Omit parentheses for method calls with arguments.".to_string(),
        ));
    }

    fn call_has_hash_value_omission(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        let args = match call.arguments() {
            Some(a) => a,
            None => return false,
        };
        let arg_list: Vec<_> = args.arguments().iter().collect();
        let last_arg = match arg_list.last() {
            Some(a) => a,
            None => return false,
        };

        // Check if last arg is a hash with value omission
        if let Some(hash) = last_arg.as_hash_node() {
            has_hash_value_omission(&hash)
        } else if let Some(kw_hash) = last_arg.as_keyword_hash_node() {
            has_keyword_hash_value_omission(&kw_hash)
        } else {
            false
        }
    }

    /// Check require_parentheses_for_hash_value_omission?
    fn require_parentheses_for_hash_value_omission(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        if !self.call_has_hash_value_omission(call) {
            return false;
        }

        // Match RuboCop's narrower allowance: keep parens only when the call
        // is the direct value of a conditional-style parent, when the parent
        // expression itself fits on a single line (mirrors
        // `node.parent&.single_line?`), or when another sibling expression
        // follows.
        let parent = self.immediate_parent();
        if matches!(
            parent,
            Some(
                ParentKind::Conditional | ParentKind::ConditionalBody | ParentKind::RescueMainBody
            )
        ) {
            return true;
        }

        if self.parent_is_single_line() {
            return true;
        }

        self.in_non_last_expression()
    }

    fn legitimate_call_with_parentheses(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        self.call_in_literals()
            || matches!(
                self.immediate_parent(),
                Some(ParentKind::When | ParentKind::WhenBody)
            )
            || self.call_with_ambiguous_arguments(call)
            || self.call_in_logical_operators()
            || self.call_in_optional_arguments()
            || self.call_in_single_line_inheritance()
            || self.allowed_multiline_call_with_parentheses(call)
            || self.allowed_chained_call_with_parentheses(call)
            || self.assignment_in_condition()
            || self.forwards_anonymous_rest_arguments(call)
    }

    fn call_in_literals(&self) -> bool {
        // RuboCop's `call_in_literals?` walks through a block parent before
        // checking the literal context, so look through any Block boundary too.
        matches!(
            self.effective_parent(),
            Some(
                ParentKind::Array
                    | ParentKind::Pair
                    | ParentKind::Range
                    | ParentKind::Splat
                    | ParentKind::KwSplat
                    | ParentKind::BlockPass
                    | ParentKind::TernaryBranch
                    | ParentKind::TernaryPredicate
            )
        )
    }

    fn call_in_logical_operators(&self) -> bool {
        self.effective_parent() == Some(ParentKind::LogicalOp)
    }

    fn call_in_optional_arguments(&self) -> bool {
        self.immediate_parent() == Some(ParentKind::OptArg)
            || self.immediate_parent() == Some(ParentKind::KwOptArg)
    }

    fn call_in_single_line_inheritance(&self) -> bool {
        self.immediate_parent() == Some(ParentKind::ClassSingleLine)
    }

    fn allowed_multiline_call_with_parentheses(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        if !self.allow_multiline {
            return false;
        }
        let call_loc = call.location();
        let (start_line, _) = self.source.offset_to_line_col(call_loc.start_offset());
        let (end_line, _) = self.source.offset_to_line_col(call_loc.end_offset());
        start_line != end_line
    }

    fn allowed_chained_call_with_parentheses(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        if !self.allow_chaining {
            return false;
        }
        has_parenthesized_ancestor_call(call)
    }

    fn call_with_ambiguous_arguments(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        self.call_with_braced_block(call)
            || self.call_in_argument_with_block(call)
            || self.call_as_argument_or_chain()
            || self.call_in_match_pattern()
            || self.hash_literal_in_arguments(call)
            || self.ambiguous_range_argument(call)
            || self.has_ambiguous_content_in_descendants(call)
            || self.call_has_block_pass(call)
    }

    fn call_with_braced_block(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        if let Some(block) = call.block() {
            if let Some(block_node) = block.as_block_node() {
                let open = block_node.opening_loc();
                let src = self.source.as_bytes();
                if open.start_offset() < src.len() && src[open.start_offset()] == b'{' {
                    return true;
                }
            }
        }
        false
    }

    fn call_in_argument_with_block(&self, _call: &ruby_prism::CallNode<'_>) -> bool {
        (self.immediate_parent() == Some(ParentKind::CallLikeBlockBody) && _call.block().is_none())
            || (_call
                .block()
                .is_some_and(|block| block.as_block_node().is_some())
                && self.block_parent_is_call_like())
    }

    fn call_as_argument_or_chain(&self) -> bool {
        self.direct_call_like_parent()
    }

    fn call_has_block_pass(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        // Check if the call has a block argument (&block)
        call.block()
            .is_some_and(|b| b.as_block_argument_node().is_some())
    }

    fn call_in_match_pattern(&self) -> bool {
        self.immediate_parent() == Some(ParentKind::MatchPattern)
    }

    fn hash_literal_in_arguments(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        let Some(args) = call.arguments() else {
            return false;
        };

        let mut has_direct_send_arg = false;
        for arg in args.arguments().iter() {
            if arg
                .as_hash_node()
                .is_some_and(|hash| hash.opening_loc().as_slice() == b"{")
            {
                return true;
            }

            has_direct_send_arg |= arg.as_call_node().is_some();
        }

        if has_direct_send_arg && call_contains_hash_literal(call) {
            return true;
        }

        false
    }

    fn ambiguous_range_argument(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        let args = match call.arguments() {
            Some(a) => a,
            None => return false,
        };
        let arg_list: Vec<_> = args.arguments().iter().collect();

        // First arg is a beginless range
        if let Some(first) = arg_list.first() {
            if let Some(range) = first.as_range_node() {
                if range.left().is_none() {
                    return true;
                }
            }
        }

        // Last arg is an endless range
        if let Some(last) = arg_list.last() {
            if let Some(range) = last.as_range_node() {
                if range.right().is_none() {
                    return true;
                }
            }
        }

        false
    }

    /// Check for forwarded args, ambiguous literals, logical operators, and blocks in descendants
    fn has_ambiguous_content_in_descendants(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        if let Some(recv) = call.receiver() {
            if is_ambiguous_descendant(&recv, self.source) {
                return true;
            }
        }

        if let Some(args) = call.arguments() {
            for arg in args.arguments().iter() {
                if is_ambiguous_descendant(&arg, self.source) {
                    return true;
                }
            }
        }
        false
    }

    fn forwards_anonymous_rest_arguments(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        if let Some(args) = call.arguments() {
            let arg_list: Vec<_> = args.arguments().iter().collect();
            if let Some(last) = arg_list.last() {
                // forwarded_restarg_type? — anonymous *
                if last
                    .as_splat_node()
                    .is_some_and(|s| s.expression().is_none())
                {
                    return true;
                }
                // Check for forwarded_kwrestarg in hash
                if let Some(kw_hash) = last.as_keyword_hash_node() {
                    for elem in kw_hash.elements().iter() {
                        if elem
                            .as_assoc_splat_node()
                            .is_some_and(|s| s.value().is_none())
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn assignment_in_condition(&self) -> bool {
        if self.parent_stack.len() >= 2 {
            let parent = self.parent_stack[self.parent_stack.len() - 1];
            let grandparent = self.parent_stack[self.parent_stack.len() - 2];
            if matches!(parent, ParentKind::Assignment | ParentKind::CallAssignedRhs)
                && matches!(
                    grandparent,
                    ParentKind::Conditional
                        | ParentKind::ConditionalBody
                        | ParentKind::When
                        | ParentKind::WhenBody
                        // Ternary `if` nodes are `conditional?` in RuboCop, so
                        // `value = call(...)` inside `cond ? ... : value = call(...)`
                        // keeps its parens via assignment_in_condition?.
                        | ParentKind::TernaryBranch
                )
            {
                return true;
            }
        }

        false
    }

    fn visit_call_common(&mut self, call: &ruby_prism::CallNode<'_>) {
        match self.enforced_style {
            "omit_parentheses" => self.check_omit_parentheses(call),
            _ => self.check_require_parentheses(call),
        }
    }

    /// Check yield node in require_parentheses mode.
    /// RuboCop aliases `on_yield` to `on_send`, so yield with args is checked.
    fn check_require_parentheses_yield(&mut self, node: &ruby_prism::YieldNode<'_>) {
        let has_parens = node.lparen_loc().is_some();
        if has_parens {
            return;
        }

        // Must have arguments
        if node.arguments().is_none() {
            return;
        }

        // AllowedMethods: check if "yield" is in the list
        if let Some(methods) = self.allowed_methods {
            if methods.iter().any(|m| m == "yield") {
                return;
            }
        }

        // AllowedPatterns: check if "yield" matches any pattern
        if let Some(patterns) = self.allowed_patterns {
            if matches_any_pattern("yield", patterns) {
                return;
            }
        }

        // IgnoreMacros: yield is always receiverless, check macro scope.
        if self.ignore_macros && self.is_macro_scope() && !self.nested_in_non_wrapper() {
            let in_included = self
                .included_macros
                .is_some_and(|macros| macros.iter().any(|m| m == "yield"));
            let in_included_patterns = self
                .included_macro_patterns
                .is_some_and(|patterns| matches_any_pattern("yield", patterns));

            if !in_included && !in_included_patterns {
                return;
            }
        }

        // Report at the yield keyword location
        let (line, column) = self
            .source
            .offset_to_line_col(node.keyword_loc().start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Use parentheses for method calls with arguments.".to_string(),
        ));
    }

    /// Check yield node in omit_parentheses mode.
    fn check_omit_parentheses_yield(&mut self, node: &ruby_prism::YieldNode<'_>) {
        let has_parens = node.lparen_loc().is_some();
        if !has_parens {
            return;
        }

        // inside_endless_method_def? — parens required in endless methods
        if self.in_endless_def && node.arguments().is_some() {
            return;
        }

        // super_call_without_arguments? — yield is not super

        // legitimate_call_with_parentheses? — check applicable sub-checks.
        // RuboCop aliases on_yield to on_send, so yield must consult the same
        // legitimate_call_with_parentheses? guards. assignment_in_condition?
        // is the one that matters most often: `lhs = yield(...) if cond` and
        // `lhs, rhs = yield(...) unless cond` keep their parens because the
        // surrounding assignment lives inside a conditional.
        if self.call_in_literals()
            || matches!(
                self.immediate_parent(),
                Some(ParentKind::When | ParentKind::WhenBody)
            )
            || self.call_in_logical_operators()
            || self.call_in_optional_arguments()
            || self.call_as_argument_or_chain()
            || self.call_in_match_pattern()
            || self.assignment_in_condition()
        {
            return;
        }

        // Check for ambiguous arguments in yield's args
        if let Some(args) = node.arguments() {
            for arg in args.arguments().iter() {
                if is_ambiguous_descendant(&arg, self.source) {
                    return;
                }
            }
        }

        let open_loc = match node.lparen_loc() {
            Some(loc) => loc,
            None => return,
        };
        let (line, column) = self.source.offset_to_line_col(open_loc.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Omit parentheses for method calls with arguments.".to_string(),
        ));
    }
}

/// Check if a hash node's LAST pair has value omission (Ruby 3.1 shorthand
/// `{foo:}`). Matches RuboCop's `last_argument.pairs.last&.value_omission?` —
/// an earlier pair being shorthand does not exempt the call when the last
/// pair is a regular `k: v`.
fn has_hash_value_omission(hash: &ruby_prism::HashNode<'_>) -> bool {
    hash.elements().iter().last().is_some_and(|elem| {
        elem.as_assoc_node()
            .is_some_and(|assoc| assoc.value().as_implicit_node().is_some())
    })
}

fn has_keyword_hash_value_omission(kw_hash: &ruby_prism::KeywordHashNode<'_>) -> bool {
    kw_hash.elements().iter().last().is_some_and(|elem| {
        elem.as_assoc_node()
            .is_some_and(|assoc| assoc.value().as_implicit_node().is_some())
    })
}

fn unwrap_parenthesized_node<'a>(node: &ruby_prism::Node<'a>) -> Option<ruby_prism::Node<'a>> {
    let mut current = node.as_parentheses_node()?.body()?;

    loop {
        if let Some(stmts) = current.as_statements_node() {
            let stmts_body = stmts.body();
            if stmts_body.len() != 1 {
                return None;
            }
            current = stmts_body.iter().next().unwrap();
            continue;
        }

        if let Some(paren) = current.as_parentheses_node() {
            current = paren.body()?;
            continue;
        }

        return Some(current);
    }
}

/// Check if a node contains a hash literal with braces (not keyword hash)
fn has_hash_literal(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(unwrapped) = unwrap_parenthesized_node(node) {
        return has_hash_literal(&unwrapped);
    }

    if let Some(hash) = node.as_hash_node() {
        if hash.opening_loc().as_slice() == b"{" {
            return true;
        }
    }
    // Recurse into descendants
    if let Some(call) = node.as_call_node() {
        if let Some(args) = call.arguments() {
            for arg in args.arguments().iter() {
                if has_hash_literal(&arg) {
                    return true;
                }
            }
        }
        if let Some(recv) = call.receiver() {
            if has_hash_literal(&recv) {
                return true;
            }
        }
    }
    if let Some(array) = node.as_array_node() {
        for elem in array.elements().iter() {
            if has_hash_literal(&elem) {
                return true;
            }
        }
    }
    if let Some(kw_hash) = node.as_keyword_hash_node() {
        for elem in kw_hash.elements().iter() {
            if has_hash_literal(&elem) {
                return true;
            }
        }
    }
    if let Some(assoc) = node.as_assoc_node() {
        if has_hash_literal(&assoc.value()) {
            return true;
        }
    }
    false
}

fn call_contains_hash_literal(call: &ruby_prism::CallNode<'_>) -> bool {
    if let Some(recv) = call.receiver() {
        if has_hash_literal(&recv) {
            return true;
        }
    }

    if let Some(args) = call.arguments() {
        for arg in args.arguments().iter() {
            if has_hash_literal(&arg) {
                return true;
            }
        }
    }

    false
}

/// Check if a CallNode has parenthesized ancestor calls in the chain
fn has_parenthesized_ancestor_call(call: &ruby_prism::CallNode<'_>) -> bool {
    let mut current = call.receiver();
    while let Some(recv) = current {
        if let Some(recv_call) = recv.as_call_node() {
            if recv_call.opening_loc().is_some() {
                return true;
            }
            current = recv_call.receiver();
        } else {
            break;
        }
    }
    false
}

fn call_child_parent_kind(
    call: &ruby_prism::CallNode<'_>,
    default_parent: ParentKind,
    child_start_offset: usize,
) -> ParentKind {
    if let Some(equal_loc) = call.equal_loc() {
        if child_start_offset > equal_loc.start_offset() {
            return if matches!(
                default_parent,
                ParentKind::Call | ParentKind::ClassConstructor
            ) {
                ParentKind::CallAssignedRhs
            } else {
                ParentKind::Assignment
            };
        }
    }

    default_parent
}

/// Check whether a single node is ambiguous in omit_parentheses style. This
/// mirrors RuboCop's per-node check inside `call_with_ambiguous_arguments?`
/// (`ambiguous_literal?`, `logical_operator?`, `:forwarded_args`, `:any_block`,
/// plus the lambda/splat/block-pass shortcuts on the immediate argument).
fn is_ambiguous_node(node: &ruby_prism::Node<'_>, source: &SourceFile) -> bool {
    if node.as_splat_node().is_some()
        || node.as_assoc_splat_node().is_some()
        || node.as_block_argument_node().is_some()
        || node.as_lambda_node().is_some()
        || node.as_block_node().is_some()
        || node.as_forwarding_arguments_node().is_some()
        || node.as_and_node().is_some()
        || node.as_or_node().is_some()
    {
        return true;
    }

    // Ternary if — has then_keyword (the `?`) but no end_keyword
    if let Some(if_node) = node.as_if_node() {
        if if_node.then_keyword_loc().is_some() && if_node.end_keyword_loc().is_none() {
            return true;
        }
    }

    // Regex slash literal (RuboCop's regexp_slash_literal? — opens with `/`)
    if let Some(regex) = node.as_regular_expression_node() {
        let bytes = source.as_bytes();
        let open = regex.opening_loc();
        if open.start_offset() < bytes.len() && bytes[open.start_offset()] == b'/' {
            return true;
        }
    }
    if let Some(regex) = node.as_interpolated_regular_expression_node() {
        let bytes = source.as_bytes();
        let open_offset = regex.opening_loc().start_offset();
        if open_offset < bytes.len() && bytes[open_offset] == b'/' {
            return true;
        }
    }

    // Numeric with sign (RuboCop's `numeric_type? && sign?`)
    if node.as_integer_node().is_some()
        || node.as_float_node().is_some()
        || node.as_rational_node().is_some()
        || node.as_imaginary_node().is_some()
    {
        let bytes = source.as_bytes();
        let start = node.location().start_offset();
        if start < bytes.len() && (bytes[start] == b'-' || bytes[start] == b'+') {
            return true;
        }
    }

    // Unary operation on non-numeric (e.g., `!foo`, `+""`, `-""`, `~bar`)
    if let Some(call) = node.as_call_node() {
        if method_dispatch_predicates::is_unary_operation(&call) {
            return true;
        }
        // RuboCop's `node.descendants.any? { type?(:any_block) }` matches calls
        // that carry a block literal (a `block` node attached to a send).
        if call.block().is_some() {
            return true;
        }
    }

    false
}

struct AmbiguousDescendantVisitor<'a> {
    source: &'a SourceFile,
    found: bool,
}

impl<'pr, 'a> Visit<'pr> for AmbiguousDescendantVisitor<'a> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        if self.found {
            return;
        }
        if is_ambiguous_node(&node, self.source) {
            self.found = true;
        }
    }

    fn visit_leaf_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        if self.found {
            return;
        }
        if is_ambiguous_node(&node, self.source) {
            self.found = true;
        }
    }
}

/// Return true if `node` or any of its descendants matches RuboCop's
/// ambiguous-content rule for `omit_parentheses`. Walks the entire subtree via
/// the Prism `Visit` trait so that descendants reachable through ranges,
/// interpolated strings/symbols/regexes, paren-grouping, etc. are all
/// considered — matching `node.descendants.any?` in rubocop-ast.
fn is_ambiguous_descendant(node: &ruby_prism::Node<'_>, source: &SourceFile) -> bool {
    if let Some(unwrapped) = unwrap_parenthesized_node(node) {
        return is_ambiguous_descendant(&unwrapped, source);
    }
    if is_ambiguous_node(node, source) {
        return true;
    }
    let mut visitor = AmbiguousDescendantVisitor {
        source,
        found: false,
    };
    visitor.visit(node);
    visitor.found
}

impl<'pr> Visit<'pr> for ParenVisitor<'_> {
    fn visit_statements_node(&mut self, node: &ruby_prism::StatementsNode<'pr>) {
        let body = node.body();
        // Multi-statement bodies wrap as `:begin` in Parser AST. Push a
        // synthetic boundary marker so an outer ConditionalBody/WhenBody
        // doesn't leak through as the grandparent of an inner assignment.
        let pushed_begin = body.len() > 1;
        if pushed_begin {
            self.push_parent(ParentKind::Begin);
        }
        for (index, stmt) in body.iter().enumerate() {
            let is_not_last = index + 1 < body.len();
            if is_not_last {
                self.non_last_expression_depth += 1;
            }
            self.visit(&stmt);
            if is_not_last {
                self.non_last_expression_depth -= 1;
            }
        }
        if pushed_begin {
            self.pop_parent();
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.visit_call_common(node);

        let is_class_constructor = is_class_constructor(node);
        let child_parent = if is_class_constructor {
            ParentKind::ClassConstructor
        } else {
            ParentKind::Call
        };

        if is_class_constructor {
            self.push_macro_scope(MacroScope::InMacroScope);
        }

        // Visit children — push Call as parent for receiver, args, and block arg
        // because in RuboCop, all these children have the call as parent node.
        // Record the call's location so `parent_is_single_line()` can answer
        // RuboCop's `node.parent&.single_line?` for nested-argument calls.
        if let Some(recv) = node.receiver() {
            self.push_parent_with_loc(child_parent, node.location());
            self.visit(&recv);
            self.pop_parent();
        }
        if let Some(args) = node.arguments() {
            for arg in args.arguments().iter() {
                self.push_parent_with_loc(
                    call_child_parent_kind(node, child_parent, arg.location().start_offset()),
                    node.location(),
                );
                self.visit(&arg);
                self.pop_parent();
            }
        }
        if let Some(block) = node.block() {
            if let Some(block_node) = block.as_block_node() {
                if is_class_constructor {
                    self.visit_block_node(&block_node);
                } else {
                    // In Parser AST, the block node inherits the enclosing
                    // expression's parent, not the send's parent. That means
                    // ordinary call-attached blocks only keep macro scope when
                    // the whole block expression is itself in macro scope.
                    let child_scope = self.call_block_child_scope();
                    let block_parent_is_call_like = self.block_parent_is_call_like();
                    let leaked_parent = if matches!(
                        self.immediate_parent(),
                        Some(
                            ParentKind::Call
                                | ParentKind::ClassConstructor
                                | ParentKind::CallAssignedRhs
                                | ParentKind::Super
                        )
                    ) {
                        self.pop_parent()
                    } else {
                        None
                    };
                    let block_loc = block_node.location();
                    let block_loc_pair = (block_loc.start_offset(), block_loc.end_offset());
                    self.push_parent_with_loc(ParentKind::Block, block_loc);
                    self.push_macro_scope(child_scope);
                    if let Some(params) = block_node.parameters() {
                        self.visit(&params);
                    }
                    if let Some(body) = block_node.body() {
                        // Reset non_last_expression_depth at block body entry
                        // (mirrors `visit_block_node`/`visit_lambda_node`).
                        let prev_non_last_expression_depth = self.non_last_expression_depth;
                        self.non_last_expression_depth = 0;
                        if block_parent_is_call_like {
                            self.visit_node_with_parent_loc(
                                &body,
                                ParentKind::CallLikeBlockBody,
                                Some(block_loc_pair),
                            );
                        } else {
                            self.visit(&body);
                        }
                        self.non_last_expression_depth = prev_non_last_expression_depth;
                    }
                    self.pop_scope();
                    self.pop_parent();
                    if let Some(parent) = leaked_parent {
                        self.push_parent(parent);
                    }
                }
            } else {
                // BlockArgumentNode (&block) — this IS a call argument
                self.push_parent(call_child_parent_kind(
                    node,
                    child_parent,
                    block.location().start_offset(),
                ));
                self.visit(&block);
                self.pop_parent();
            }
        }

        if is_class_constructor {
            self.pop_scope();
        }
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        // Check if single-line
        let (start_line, _) = self
            .source
            .offset_to_line_col(node.location().start_offset());
        let (end_line, _) = self.source.offset_to_line_col(node.location().end_offset());
        let is_single_line = start_line == end_line;

        if let Some(superclass) = node.superclass() {
            if is_single_line {
                self.push_parent(ParentKind::ClassSingleLine);
            }
            self.visit(&superclass);
            if is_single_line {
                self.pop_parent();
            }
        }

        self.push_macro_scope(MacroScope::InMacroScope);
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.pop_scope();
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        self.push_macro_scope(MacroScope::InMacroScope);
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.pop_scope();
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        self.push_macro_scope(MacroScope::InMacroScope);
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.pop_scope();
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let is_endless = node.end_keyword_loc().is_none() && node.equal_loc().is_some();
        let prev_endless = self.in_endless_def;
        if is_endless {
            self.in_endless_def = true;
        }

        self.push_macro_scope(MacroScope::NotMacroScope);
        // Visit parameters
        if let Some(params) = node.parameters() {
            self.visit_parameters_node(&params);
        }
        if let Some(body) = node.body() {
            let prev_non_last_expression_depth = self.non_last_expression_depth;
            self.non_last_expression_depth = 0;
            self.visit(&body);
            self.non_last_expression_depth = prev_non_last_expression_depth;
        }
        self.pop_scope();
        self.in_endless_def = prev_endless;
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        // Parser gives expressions inside a `block` node the block itself as
        // the direct parent.  When Prism walks a chained receiver block
        // (`expect do ... end.to ...`), the surrounding `Call` would
        // otherwise leak into the body and make inner calls look like chained
        // arguments.
        let block_parent_is_call_like = self.block_parent_is_call_like();
        let leaked_parent = if matches!(
            self.immediate_parent(),
            Some(
                ParentKind::Call
                    | ParentKind::ClassConstructor
                    | ParentKind::CallAssignedRhs
                    | ParentKind::Super
            )
        ) {
            self.pop_parent()
        } else {
            None
        };

        let block_loc = node.location();
        let block_loc_pair = (block_loc.start_offset(), block_loc.end_offset());
        self.push_parent_with_loc(ParentKind::Block, block_loc);
        let child_scope = self.wrapper_child_scope();
        self.push_macro_scope(child_scope);
        if let Some(params) = node.parameters() {
            self.visit(&params);
        }
        if let Some(body) = node.body() {
            // Reset non_last_expression_depth so an outer non-last sibling
            // (e.g., the `if` above a `.map do ... end`) doesn't make
            // `Foo.bar(value:)` inside the block falsely pass
            // `require_parentheses_for_hash_value_omission?` via the
            // `!last_expression?` branch. RuboCop only checks the call's own
            // (or its assignment parent's) right sibling, which restarts
            // inside each block body.
            let prev_non_last_expression_depth = self.non_last_expression_depth;
            self.non_last_expression_depth = 0;
            if block_parent_is_call_like {
                self.visit_node_with_parent_loc(
                    &body,
                    ParentKind::CallLikeBlockBody,
                    Some(block_loc_pair),
                );
            } else {
                self.visit(&body);
            }
            self.non_last_expression_depth = prev_non_last_expression_depth;
        }
        self.pop_scope();
        self.pop_parent();

        if let Some(parent) = leaked_parent {
            self.push_parent(parent);
        }
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        // In Parser AST, `-> { ... }` is `(block (send nil :lambda) ...)`.
        // RuboCop's `in_macro_scope?` does NOT list `block` as a wrapper —
        // only `class_constructor?` blocks propagate macro scope.  Since a
        // lambda literal is never a class constructor, its body only inherits
        // macro scope when the lambda expression itself is in macro scope
        // (i.e. not nested under a non-wrapper parent such as a call's
        // arguments).  Use `call_block_child_scope()` so that lambdas passed
        // as arguments (`scope :x, -> { where ... }`) break macro scope,
        // while lambdas inside wrapper blocks (`subject { -> { get :idx } }`)
        // preserve it.
        //
        // For omit_parentheses, lambdas mirror block-on-call semantics: when
        // the lambda is a direct argument of a call/super/yield (the lambda's
        // Parser parent is a `block` whose own parent is the call), inner
        // calls in the lambda body fall under RuboCop's
        // `call_in_argument_with_block?` and keep their parentheses. Pop the
        // surrounding call-like parent and mark the body with
        // `CallLikeBlockBody` for that case, matching `visit_block_node`.
        let child_scope = self.call_block_child_scope();
        let block_parent_is_call_like = self.block_parent_is_call_like();
        let leaked_parent = if matches!(
            self.immediate_parent(),
            Some(
                ParentKind::Call
                    | ParentKind::ClassConstructor
                    | ParentKind::CallAssignedRhs
                    | ParentKind::Super
            )
        ) {
            self.pop_parent()
        } else {
            None
        };

        let block_loc = node.location();
        let block_loc_pair = (block_loc.start_offset(), block_loc.end_offset());
        self.push_parent_with_loc(ParentKind::Block, block_loc);
        self.push_macro_scope(child_scope);
        if let Some(body) = node.body() {
            // Same reset as `visit_block_node`: each lambda body computes
            // value-omission siblings independently, so the surrounding
            // expression's non-last-sibling depth must not leak in.
            let prev_non_last_expression_depth = self.non_last_expression_depth;
            self.non_last_expression_depth = 0;
            if block_parent_is_call_like {
                self.visit_node_with_parent_loc(
                    &body,
                    ParentKind::CallLikeBlockBody,
                    Some(block_loc_pair),
                );
            } else {
                self.visit(&body);
            }
            self.non_last_expression_depth = prev_non_last_expression_depth;
        }
        self.pop_scope();
        self.pop_parent();

        if let Some(parent) = leaked_parent {
            self.push_parent(parent);
        }
    }

    fn visit_yield_node(&mut self, node: &ruby_prism::YieldNode<'pr>) {
        // RuboCop aliases on_yield to on_send for this cop
        match self.enforced_style {
            "omit_parentheses" => self.check_omit_parentheses_yield(node),
            _ => self.check_require_parentheses_yield(node),
        }

        // Visit arguments as children
        if let Some(args) = node.arguments() {
            self.push_parent(ParentKind::Call);
            for arg in args.arguments().iter() {
                self.visit(&arg);
            }
            self.pop_parent();
        }
    }

    fn visit_super_node(&mut self, node: &ruby_prism::SuperNode<'pr>) {
        if let Some(args) = node.arguments() {
            self.push_parent(ParentKind::Super);
            for arg in args.arguments().iter() {
                self.visit(&arg);
            }
            self.pop_parent();
        }

        if let Some(block) = node.block() {
            if let Some(block_node) = block.as_block_node() {
                let child_scope = self.call_block_child_scope();
                let block_parent_is_call_like = self.block_parent_is_call_like();
                let leaked_parent = if matches!(
                    self.immediate_parent(),
                    Some(
                        ParentKind::Call
                            | ParentKind::ClassConstructor
                            | ParentKind::CallAssignedRhs
                            | ParentKind::Super
                    )
                ) {
                    self.pop_parent()
                } else {
                    None
                };

                let block_loc = block_node.location();
                let block_loc_pair = (block_loc.start_offset(), block_loc.end_offset());
                self.push_parent_with_loc(ParentKind::Block, block_loc);
                self.push_macro_scope(child_scope);
                if let Some(params) = block_node.parameters() {
                    self.visit(&params);
                }
                if let Some(body) = block_node.body() {
                    // Reset non_last_expression_depth at block body entry
                    // (mirrors `visit_block_node`/`visit_lambda_node`).
                    let prev_non_last_expression_depth = self.non_last_expression_depth;
                    self.non_last_expression_depth = 0;
                    if block_parent_is_call_like {
                        self.visit_node_with_parent_loc(
                            &body,
                            ParentKind::CallLikeBlockBody,
                            Some(block_loc_pair),
                        );
                    } else {
                        self.visit(&body);
                    }
                    self.non_last_expression_depth = prev_non_last_expression_depth;
                }
                self.pop_scope();
                self.pop_parent();

                if let Some(parent) = leaked_parent {
                    self.push_parent(parent);
                }
            } else {
                self.push_parent(ParentKind::Super);
                self.visit(&block);
                self.pop_parent();
            }
        }
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        let has_rescue_or_ensure = node.rescue_clause().is_some() || node.ensure_clause().is_some();

        // Capture `nested_in_non_wrapper` BEFORE popping the leaked parent so
        // the begin's wrapper-vs-non-wrapper decision still sees the surrounding
        // setter/chained call. Otherwise `Foo.bar = begin ...; raise "x"; end`
        // would treat the begin as a fresh wrapper and silently exempt inner
        // receiverless calls as macros.
        let begin_nested_in_non_wrapper = self.nested_in_non_wrapper();

        // In Parser AST, statements inside `begin ... end` (with or without
        // rescue/ensure) have the (kw)begin node as their direct parent, NOT
        // the call that chains off the begin expression. Pop and restore an
        // outer Call/Super/CallAssignedRhs/ClassConstructor so that
        // `call_as_argument_or_chain?` doesn't leak through, mirroring
        // `visit_block_node`'s call-like-parent handling.
        let leaked_parent = if matches!(
            self.immediate_parent(),
            Some(
                ParentKind::Call
                    | ParentKind::ClassConstructor
                    | ParentKind::CallAssignedRhs
                    | ParentKind::Super
            )
        ) {
            self.pop_parent()
        } else {
            None
        };

        if has_rescue_or_ensure {
            // In Parser AST, `begin; foo; rescue; bar; end` produces:
            //   (kwbegin (rescue (send nil :foo) (resbody nil nil (send nil :bar)) nil))
            // The `rescue` node sits between `kwbegin` and all children.
            // RuboCop's `in_macro_scope?` does NOT list `rescue` or `ensure` as
            // wrappers, so nothing inside a begin-with-rescue gets macro scope.
            self.push_macro_scope(MacroScope::NotMacroScope);
            if let Some(stmts) = node.statements() {
                self.non_last_expression_depth += 1;
                self.visit_statements_with_parent(&stmts, ParentKind::RescueMainBody);
                self.non_last_expression_depth -= 1;
            }
            if let Some(rescue_clause) = node.rescue_clause() {
                self.visit_rescue_node(&rescue_clause);
            }
            if let Some(else_clause) = node.else_clause() {
                self.visit_else_node(&else_clause);
            }
            if let Some(ensure_clause) = node.ensure_clause() {
                self.visit_ensure_node(&ensure_clause);
            }
            self.pop_scope();
        } else {
            // Pure `begin...end` (no rescue/ensure) — `kwbegin` is a wrapper
            // in RuboCop's `in_macro_scope?`, but only when the whole begin
            // expression is itself in macro scope.
            let child_scope = if begin_nested_in_non_wrapper {
                MacroScope::NotMacroScope
            } else {
                self.wrapper_child_scope()
            };
            self.push_macro_scope(child_scope);
            ruby_prism::visit_begin_node(self, node);
            self.pop_scope();
        }

        if let Some(parent) = leaked_parent {
            self.push_parent(parent);
        }
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        // Check if this is a ternary: has then_keyword (the `?`) but no end_keyword
        let is_ternary = node.then_keyword_loc().is_some() && node.end_keyword_loc().is_none();

        // `if`/`unless` conditions are not wrapper context for macros.
        // Ternary predicates also count as ternary literal context for
        // omit-parentheses checks, so track them separately from branches.
        self.push_parent(if is_ternary {
            ParentKind::TernaryPredicate
        } else {
            ParentKind::Conditional
        });
        self.visit(&node.predicate());
        self.pop_parent();

        // `if`/ternary branches only inherit macro scope when the whole `if`
        // expression is itself in macro scope.
        let child_scope = if self.nested_in_non_wrapper() {
            MacroScope::NotMacroScope
        } else {
            self.wrapper_child_scope()
        };

        if let Some(stmts) = node.statements() {
            self.push_macro_scope(child_scope);
            if is_ternary {
                self.push_parent(ParentKind::TernaryBranch);
                self.visit_statements_node(&stmts);
                self.pop_parent();
            } else {
                self.visit_statements_with_parent(&stmts, ParentKind::ConditionalBody);
            }
            self.pop_scope();
        }
        if let Some(subsequent) = node.subsequent() {
            self.push_macro_scope(child_scope);
            if is_ternary {
                self.push_parent(ParentKind::TernaryBranch);
                self.visit(&subsequent);
                self.pop_parent();
            } else if let Some(else_node) = subsequent.as_else_node() {
                if let Some(stmts) = else_node.statements() {
                    self.visit_statements_with_parent(&stmts, ParentKind::ConditionalBody);
                }
            } else {
                self.visit(&subsequent);
            }
            self.pop_scope();
        }
    }

    fn visit_parentheses_node(&mut self, node: &ruby_prism::ParenthesesNode<'pr>) {
        self.push_parent(ParentKind::Grouped);
        ruby_prism::visit_parentheses_node(self, node);
        self.pop_parent();
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        self.push_parent(ParentKind::Conditional);
        self.visit(&node.predicate());
        self.pop_parent();

        let child_scope = if self.nested_in_non_wrapper() {
            MacroScope::NotMacroScope
        } else {
            self.wrapper_child_scope()
        };

        if let Some(stmts) = node.statements() {
            self.push_macro_scope(child_scope);
            self.visit_statements_with_parent(&stmts, ParentKind::ConditionalBody);
            self.pop_scope();
        }
        if let Some(consequent) = node.else_clause() {
            self.push_macro_scope(child_scope);
            if let Some(stmts) = consequent.statements() {
                self.visit_statements_with_parent(&stmts, ParentKind::ConditionalBody);
            }
            self.pop_scope();
        }
    }

    // Track parent context for omit_parentheses checks
    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        self.push_parent(ParentKind::Array);
        for elem in node.elements().iter() {
            self.visit(&elem);
        }
        self.pop_parent();
    }

    fn visit_assoc_node(&mut self, node: &ruby_prism::AssocNode<'pr>) {
        self.push_parent(ParentKind::Pair);
        self.visit(&node.key());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_range_node(&mut self, node: &ruby_prism::RangeNode<'pr>) {
        self.push_parent(ParentKind::Range);
        if let Some(left) = node.left() {
            self.visit(&left);
        }
        if let Some(right) = node.right() {
            self.visit(&right);
        }
        self.pop_parent();
    }

    fn visit_splat_node(&mut self, node: &ruby_prism::SplatNode<'pr>) {
        self.push_parent(ParentKind::Splat);
        if let Some(expr) = node.expression() {
            self.visit(&expr);
        }
        self.pop_parent();
    }

    fn visit_assoc_splat_node(&mut self, node: &ruby_prism::AssocSplatNode<'pr>) {
        self.push_parent(ParentKind::KwSplat);
        if let Some(value) = node.value() {
            self.visit(&value);
        }
        self.pop_parent();
    }

    fn visit_block_argument_node(&mut self, node: &ruby_prism::BlockArgumentNode<'pr>) {
        self.push_parent(ParentKind::BlockPass);
        if let Some(expr) = node.expression() {
            self.visit(&expr);
        }
        self.pop_parent();
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        self.push_parent(ParentKind::LogicalOp);
        self.visit(&node.left());
        self.visit(&node.right());
        self.pop_parent();
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        self.push_parent(ParentKind::LogicalOp);
        self.visit(&node.left());
        self.visit(&node.right());
        self.pop_parent();
    }

    fn visit_optional_parameter_node(&mut self, node: &ruby_prism::OptionalParameterNode<'pr>) {
        self.push_parent(ParentKind::OptArg);
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_optional_keyword_parameter_node(
        &mut self,
        node: &ruby_prism::OptionalKeywordParameterNode<'pr>,
    ) {
        self.push_parent(ParentKind::KwOptArg);
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_match_required_node(&mut self, node: &ruby_prism::MatchRequiredNode<'pr>) {
        self.push_parent(ParentKind::MatchPattern);
        self.visit(&node.value());
        self.pop_parent();
        self.visit(&node.pattern());
    }

    fn visit_match_predicate_node(&mut self, node: &ruby_prism::MatchPredicateNode<'pr>) {
        self.push_parent(ParentKind::MatchPattern);
        self.visit(&node.value());
        self.pop_parent();
        self.visit(&node.pattern());
    }

    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'pr>) {
        // `case`/`when` are NOT wrappers in RuboCop's in_macro_scope?.
        // Push Other to prevent class-like scope from leaking through.
        //
        // In Parser AST, the `else` branch's call has the `case` node as its
        // direct parent, NOT the chained call that sits *outside* the case
        // expression (e.g. `case ... end.transform_values!`). Pop and restore
        // an outer Call/Super/CallAssignedRhs/ClassConstructor so
        // `call_as_argument_or_chain?` doesn't leak through.
        let leaked_parent = if matches!(
            self.immediate_parent(),
            Some(
                ParentKind::Call
                    | ParentKind::ClassConstructor
                    | ParentKind::CallAssignedRhs
                    | ParentKind::Super
            )
        ) {
            self.pop_parent()
        } else {
            None
        };

        self.push_macro_scope(MacroScope::NotMacroScope);
        ruby_prism::visit_case_node(self, node);
        self.pop_scope();

        if let Some(parent) = leaked_parent {
            self.push_parent(parent);
        }
    }

    fn visit_case_match_node(&mut self, node: &ruby_prism::CaseMatchNode<'pr>) {
        // `case`/`in` (pattern matching) is NOT a wrapper in in_macro_scope?.
        let leaked_parent = if matches!(
            self.immediate_parent(),
            Some(
                ParentKind::Call
                    | ParentKind::ClassConstructor
                    | ParentKind::CallAssignedRhs
                    | ParentKind::Super
            )
        ) {
            self.pop_parent()
        } else {
            None
        };

        self.push_macro_scope(MacroScope::NotMacroScope);
        ruby_prism::visit_case_match_node(self, node);
        self.pop_scope();

        if let Some(parent) = leaked_parent {
            self.push_parent(parent);
        }
    }

    fn visit_pre_execution_node(&mut self, node: &ruby_prism::PreExecutionNode<'pr>) {
        // `BEGIN { }` (`preexe`) is NOT a wrapper in in_macro_scope?.
        self.push_macro_scope(MacroScope::NotMacroScope);
        ruby_prism::visit_pre_execution_node(self, node);
        self.pop_scope();
    }

    fn visit_post_execution_node(&mut self, node: &ruby_prism::PostExecutionNode<'pr>) {
        // `END { }` (`postexe`) is NOT a wrapper in in_macro_scope?.
        self.push_macro_scope(MacroScope::NotMacroScope);
        ruby_prism::visit_post_execution_node(self, node);
        self.pop_scope();
    }

    fn visit_when_node(&mut self, node: &ruby_prism::WhenNode<'pr>) {
        // Push When before visiting conditions so they have correct parent context
        self.push_parent(ParentKind::When);
        for cond in node.conditions().iter() {
            self.visit(&cond);
        }
        self.pop_parent();

        // Push When before visiting statements so calls in the when body
        // have When as parent (matching RuboCop's node.parent.when_type? check)
        if let Some(stmts) = node.statements() {
            self.visit_statements_with_parent(&stmts, ParentKind::WhenBody);
        }
    }

    fn visit_in_node(&mut self, node: &ruby_prism::InNode<'pr>) {
        self.visit(&node.pattern());
        if let Some(stmts) = node.statements() {
            self.visit_statements_with_parent(&stmts, ParentKind::InPatternBody);
        }
    }

    fn visit_constant_path_node(&mut self, node: &ruby_prism::ConstantPathNode<'pr>) {
        // The child (left side of ::) gets ConstantPath as parent context
        if let Some(parent_node) = node.parent() {
            self.push_parent(ParentKind::ConstantPath);
            self.visit(&parent_node);
            self.pop_parent();
        }
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        // In Parser AST, `dstr` is NOT a wrapper in `in_macro_scope?`.
        // Push Interpolation parent so nested calls break macro scope.
        let prev = self.in_interpolation;
        self.in_interpolation = true;
        self.push_parent(ParentKind::Interpolation);
        for part in node.parts().iter() {
            self.visit(&part);
        }
        self.pop_parent();
        self.in_interpolation = prev;
    }

    fn visit_interpolated_symbol_node(&mut self, node: &ruby_prism::InterpolatedSymbolNode<'pr>) {
        let prev = self.in_interpolation;
        self.in_interpolation = true;
        self.push_parent(ParentKind::Interpolation);
        for part in node.parts().iter() {
            self.visit(&part);
        }
        self.pop_parent();
        self.in_interpolation = prev;
    }

    fn visit_interpolated_regular_expression_node(
        &mut self,
        node: &ruby_prism::InterpolatedRegularExpressionNode<'pr>,
    ) {
        self.push_parent(ParentKind::Interpolation);
        for part in node.parts().iter() {
            self.visit(&part);
        }
        self.pop_parent();
    }

    fn visit_interpolated_x_string_node(
        &mut self,
        node: &ruby_prism::InterpolatedXStringNode<'pr>,
    ) {
        self.push_parent(ParentKind::Interpolation);
        for part in node.parts().iter() {
            self.visit(&part);
        }
        self.pop_parent();
    }

    // Track assignment context
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_instance_variable_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_class_variable_write_node(&mut self, node: &ruby_prism::ClassVariableWriteNode<'pr>) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_global_variable_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'pr>) {
        self.visit_constant_path_node(&node.target());
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        for left in node.lefts().iter() {
            self.visit(&left);
        }
        if let Some(rest) = node.rest() {
            self.visit(&rest);
        }

        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        for right in node.rights().iter() {
            self.visit(&right);
        }
        self.visit(&node.value());
        self.pop_parent();
    }

    // Operator assignment nodes (+=, -=, etc.) — RHS is Assignment context
    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_instance_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOperatorWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_class_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOperatorWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_global_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOperatorWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_constant_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantOperatorWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_constant_path_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOperatorWriteNode<'pr>,
    ) {
        self.visit_constant_path_node(&node.target());
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_call_operator_write_node(&mut self, node: &ruby_prism::CallOperatorWriteNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            self.push_parent(ParentKind::Call);
            self.visit(&receiver);
            self.pop_parent();
        }
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            self.push_parent(ParentKind::Call);
            self.visit(&receiver);
            self.pop_parent();
        }
        if let Some(args) = node.arguments() {
            self.push_parent(ParentKind::Call);
            for arg in args.arguments().iter() {
                self.visit(&arg);
            }
            self.pop_parent();
        }
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    // ||= and &&= nodes — RHS is Assignment context
    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_instance_variable_or_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOrWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_instance_variable_and_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableAndWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_class_variable_or_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOrWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_class_variable_and_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableAndWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_global_variable_or_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOrWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_global_variable_and_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableAndWriteNode<'pr>,
    ) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_constant_or_write_node(&mut self, node: &ruby_prism::ConstantOrWriteNode<'pr>) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_constant_and_write_node(&mut self, node: &ruby_prism::ConstantAndWriteNode<'pr>) {
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_constant_path_or_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOrWriteNode<'pr>,
    ) {
        self.visit_constant_path_node(&node.target());
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_constant_path_and_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathAndWriteNode<'pr>,
    ) {
        self.visit_constant_path_node(&node.target());
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            self.push_parent(ParentKind::Call);
            self.visit(&receiver);
            self.pop_parent();
        }
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            self.push_parent(ParentKind::Call);
            self.visit(&receiver);
            self.pop_parent();
        }
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            self.push_parent(ParentKind::Call);
            self.visit(&receiver);
            self.pop_parent();
        }
        if let Some(args) = node.arguments() {
            self.push_parent(ParentKind::Call);
            for arg in args.arguments().iter() {
                self.visit(&arg);
            }
            self.pop_parent();
        }
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            self.push_parent(ParentKind::Call);
            self.visit(&receiver);
            self.pop_parent();
        }
        if let Some(args) = node.arguments() {
            self.push_parent(ParentKind::Call);
            for arg in args.arguments().iter() {
                self.visit(&arg);
            }
            self.pop_parent();
        }
        self.push_parent_with_loc(ParentKind::Assignment, node.location());
        self.visit(&node.value());
        self.pop_parent();
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        // `while`/`until`/`for` are NOT wrappers in RuboCop's in_macro_scope?.
        self.push_macro_scope(MacroScope::NotMacroScope);
        self.push_parent(ParentKind::Conditional);
        self.visit(&node.predicate());
        self.pop_parent();
        if let Some(stmts) = node.statements() {
            self.visit_statements_node(&stmts);
        }
        self.pop_scope();
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        self.push_macro_scope(MacroScope::NotMacroScope);
        self.push_parent(ParentKind::Conditional);
        self.visit(&node.predicate());
        self.pop_parent();
        if let Some(stmts) = node.statements() {
            self.visit_statements_node(&stmts);
        }
        self.pop_scope();
    }

    fn visit_for_node(&mut self, node: &ruby_prism::ForNode<'pr>) {
        self.push_macro_scope(MacroScope::NotMacroScope);
        ruby_prism::visit_for_node(self, node);
        self.pop_scope();
    }

    fn visit_return_node(&mut self, node: &ruby_prism::ReturnNode<'pr>) {
        self.push_parent(ParentKind::FlowControl);
        ruby_prism::visit_return_node(self, node);
        self.pop_parent();
    }

    fn visit_break_node(&mut self, node: &ruby_prism::BreakNode<'pr>) {
        self.push_parent(ParentKind::FlowControl);
        ruby_prism::visit_break_node(self, node);
        self.pop_parent();
    }

    fn visit_next_node(&mut self, node: &ruby_prism::NextNode<'pr>) {
        self.push_parent(ParentKind::FlowControl);
        ruby_prism::visit_next_node(self, node);
        self.pop_parent();
    }

    fn visit_rescue_modifier_node(&mut self, node: &ruby_prism::RescueModifierNode<'pr>) {
        // In Parser AST, `foo rescue bar` wraps `foo` in a rescue node.
        // RuboCop's `in_macro_scope?` does NOT list `rescue` as a wrapper,
        // so calls inside a rescue modifier are NOT in macro scope.
        self.push_macro_scope(MacroScope::NotMacroScope);
        self.visit(&node.expression());
        self.visit(&node.rescue_expression());
        self.pop_scope();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::config::load_config;
    use crate::cop::CopConfig;
    use crate::testutil::{
        assert_cop_no_offenses_full_with_config, assert_cop_offenses_full_with_config,
        run_cop_full, run_cop_full_with_config,
    };

    crate::cop_fixture_tests!(
        MethodCallWithArgsParentheses,
        "cops/style/method_call_with_args_parentheses"
    );
    crate::cop_variant_fixture_tests!(
        MethodCallWithArgsParentheses,
        "cops/style/method_call_with_args_parentheses",
        omit_parentheses
    );

    fn omit_parentheses_config() -> CopConfig {
        use std::collections::HashMap;

        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        }
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "nitrocop_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn variant_test_config_yaml(path: &Path) -> String {
        format!(
            "inherit_from: {}\n\nStyle/MethodCallWithArgsParentheses:\n  EnforcedStyle: omit_parentheses\n",
            path.display()
        )
    }

    #[test]
    fn omit_parentheses_variant_offense_fixture() {
        assert_cop_offenses_full_with_config(
            &MethodCallWithArgsParentheses,
            include_bytes!(
                "../../../tests/fixtures/cops/style/method_call_with_args_parentheses/omit_parentheses_offense.rb"
            ),
            omit_parentheses_config(),
        );
    }

    #[test]
    fn omit_parentheses_variant_no_offense_fixture() {
        assert_cop_no_offenses_full_with_config(
            &MethodCallWithArgsParentheses,
            include_bytes!(
                "../../../tests/fixtures/cops/style/method_call_with_args_parentheses/omit_parentheses_no_offense.rb"
            ),
            omit_parentheses_config(),
        );
    }

    #[test]
    fn explicit_config_path_ignores_nested_rubocop_overrides() {
        let temp_dir = unique_temp_dir("mcwap_explicit_config");
        let repo_dir = temp_dir.join("repo");
        let source_path = repo_dir.join("sub/test.rb");
        let config_path = repo_dir.join("custom.yml");
        let nested_config_path = repo_dir.join("sub/.rubocop.yml");
        let baseline =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bench/corpus/baseline_rubocop.yml");

        write_file(
            &config_path,
            &variant_test_config_yaml(&baseline.canonicalize().unwrap()),
        );
        write_file(
            &nested_config_path,
            "Style/MethodCallWithArgsParentheses:\n  AllowParenthesesInMultilineCall: true\n",
        );
        write_file(&source_path, "foo(\n  bar: 1\n)\n");

        let config = load_config(Some(&config_path), Some(&source_path), None).unwrap();
        assert!(
            !config.has_dir_overrides(),
            "explicit --config should ignore nested .rubocop.yml overrides"
        );
        let cop_config =
            config.cop_config_for_file("Style/MethodCallWithArgsParentheses", &source_path);
        assert!(
            !cop_config.get_bool("AllowParenthesesInMultilineCall", false),
            "nested override must not leak into an explicit config load"
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn operators_are_ignored() {
        let source = b"x = 1 + 2\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn method_without_args_is_ok() {
        let source = b"foo.bar\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn receiverless_in_class_body_is_macro() {
        let source = b"class Foo\n  bar :baz\nend\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(diags.is_empty(), "Macro in class body should be ignored");
    }

    #[test]
    fn receiverless_in_method_body_is_not_macro() {
        let source = b"def foo\n  bar 1, 2\nend\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert_eq!(
            diags.len(),
            1,
            "Receiverless call inside method should be flagged"
        );
    }

    #[test]
    fn receiverless_in_module_body_is_macro() {
        let source = b"module Foo\n  bar :baz\nend\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(diags.is_empty(), "Macro in module body should be ignored");
    }

    #[test]
    fn receiverless_at_top_level_is_macro() {
        let source = b"puts 'hello'\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(
            diags.is_empty(),
            "Receiverless call at top level should be treated as macro"
        );
    }

    #[test]
    fn macro_in_block_inside_class() {
        let source = b"class Foo\n  concern do\n    bar :baz\n  end\nend\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(
            diags.is_empty(),
            "Macro in block inside class should be ignored"
        );
    }

    #[test]
    fn omit_parentheses_flags_parens() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo.bar(1)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(diags.len(), 1, "Should flag parens with omit_parentheses");
        assert!(diags[0].message.contains("Omit parentheses"));
    }

    #[test]
    fn omit_parentheses_allows_no_parens() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo.bar 1\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag calls without parens in omit_parentheses"
        );
    }

    #[test]
    fn omit_accepts_parens_in_array() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"[foo.bar(1)]\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens inside array literal");
    }

    #[test]
    fn omit_accepts_parens_in_logical_ops() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(a) && bar(b)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens in logical operator context"
        );
    }

    #[test]
    fn omit_accepts_parens_in_chained_calls() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo().bar(3).wait(4).it\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens in chained calls (not last)"
        );
    }

    #[test]
    fn omit_accepts_parens_in_default_arg() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def foo(arg = default(42))\n  nil\nend\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens in default argument value"
        );
    }

    #[test]
    fn omit_accepts_parens_with_splat() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(*args)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens with splat args");
    }

    #[test]
    fn omit_accepts_parens_with_block_pass() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(&block)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens with block pass");
    }

    #[test]
    fn omit_accepts_parens_with_braced_block() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(1) { 2 }\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens with braced block");
    }

    #[test]
    fn omit_accepts_parens_for_single_statement_block_body_in_chained_block() {
        let source = b"foo.bar { |x| baz(x) }.qux\n";
        let diags = run_cop_full_with_config(
            &MethodCallWithArgsParentheses,
            source,
            omit_parentheses_config(),
        );
        assert!(
            diags.is_empty(),
            "Should allow parens when the direct block parent is chained into another call"
        );
    }

    #[test]
    fn omit_flags_parens_for_single_statement_block_body_without_outer_call() {
        let source = b"foo.bar { |x| baz(x) }\n";
        let diags = run_cop_full_with_config(
            &MethodCallWithArgsParentheses,
            source,
            omit_parentheses_config(),
        );
        assert_eq!(
            diags.len(),
            1,
            "Should still flag parens when the block itself has no direct outer call parent"
        );
    }

    #[test]
    fn omit_accepts_parens_for_super_arguments() {
        let source = b"super(errors.local_attribute(attribute))\n";
        let diags = run_cop_full_with_config(
            &MethodCallWithArgsParentheses,
            source,
            omit_parentheses_config(),
        );
        assert!(
            diags.is_empty(),
            "Should allow parens for calls nested in super arguments"
        );
    }

    #[test]
    fn omit_accepts_parens_for_call_with_block_used_as_setter_rhs() {
        let source = b"self.result = run_callbacks(:execute) do\n  execute\nend\n";
        let diags = run_cop_full_with_config(
            &MethodCallWithArgsParentheses,
            source,
            omit_parentheses_config(),
        );
        assert!(
            diags.is_empty(),
            "Should allow parens when a call-with-block is the RHS of a setter call"
        );
    }

    #[test]
    fn omit_accepts_parens_with_hash_literal() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"top.test({foo: :bar})\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens with hash literal arg"
        );
    }

    #[test]
    fn omit_accepts_parens_with_unary_arg() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(-1)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens with unary minus arg");
    }

    #[test]
    fn omit_accepts_parens_with_regex() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(/regexp/)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens with regex arg");
    }

    #[test]
    fn omit_accepts_parens_with_range() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"1..limit(n)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens inside range literal");
    }

    #[test]
    fn omit_accepts_parens_in_ternary() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo.include?(bar) ? bar : quux\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens in ternary condition");
    }

    #[test]
    fn omit_accepts_parens_in_when_clause() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"case condition\nwhen do_something(arg)\nend\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens in when clause");
    }

    #[test]
    fn omit_accepts_parens_in_single_statement_when_body() {
        let source = b"case condition\nwhen :match then do_something(arg)\nend\n";
        let diags = run_cop_full_with_config(
            &MethodCallWithArgsParentheses,
            source,
            omit_parentheses_config(),
        );
        assert!(
            diags.is_empty(),
            "Should allow parens in single-statement when bodies"
        );
    }

    #[test]
    fn omit_flags_assignment_rhs_in_multi_statement_when_body() {
        let source = b"case condition\nwhen :match\n  keypair = KeyPair.from_public_key(asset.issuer.value)\n  [:alphanum4, asset.code, keypair, amount]\nend\n";
        let diags = run_cop_full_with_config(
            &MethodCallWithArgsParentheses,
            source,
            omit_parentheses_config(),
        );
        assert_eq!(
            diags.len(),
            1,
            "Should flag assignment RHS calls in multi-statement when bodies"
        );
    }

    #[test]
    fn omit_flags_assignment_rhs_in_multi_statement_if_branch() {
        let source = b"if cond\n  prepare\n  value = lookup(arg)\nend\n";
        let diags = run_cop_full_with_config(
            &MethodCallWithArgsParentheses,
            source,
            omit_parentheses_config(),
        );
        assert_eq!(
            diags.len(),
            1,
            "Should flag assignment RHS calls in multi-statement conditional branches"
        );
    }

    #[test]
    fn omit_flags_assignment_rhs_inside_single_statement_begin_branch() {
        let source = b"if cond\n  begin\n    value = lookup(arg)\n  rescue StandardError\n    nil\n  end\nend\n";
        let diags = run_cop_full_with_config(
            &MethodCallWithArgsParentheses,
            source,
            omit_parentheses_config(),
        );
        assert_eq!(
            diags.len(),
            1,
            "Should not inherit the conditional parent through a single begin/rescue wrapper"
        );
    }

    #[test]
    fn omit_accepts_parens_in_endless_def() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def x() = foo(y)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens in endless method def"
        );
    }

    #[test]
    fn omit_accepts_parens_before_constant_resolution() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"do_something(arg)::CONST\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens before constant resolution"
        );
    }

    #[test]
    fn omit_accepts_parens_as_method_arg() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"top.test 1, 2, foo: bar(3)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens for calls used as method args"
        );
    }

    #[test]
    fn omit_accepts_parenthesized_ambiguous_descendant() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def languages\n  Array(foo((bar || [])))\nend\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens when ambiguity is hidden behind grouping parentheses"
        );
    }

    #[test]
    fn omit_accepts_hash_descendant_inside_direct_send_argument() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(bar(({a: 1})))\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens when a direct send argument contains a grouped hash descendant"
        );
    }

    #[test]
    fn omit_flags_parenthesized_direct_send_with_hash_descendant() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo((bar({a: 1})))\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should still flag grouped direct arguments even when they wrap a send with a hash descendant"
        );
    }

    #[test]
    fn omit_flags_nested_hash_inside_array_argument() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo([{a: 1}])\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag grouped container arguments with nested hash literals"
        );
    }

    #[test]
    fn omit_flags_nested_hash_inside_keyword_hash_argument() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(k: {a: 1})\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag keyword-hash wrappers with nested hash literals"
        );
    }

    #[test]
    fn omit_accepts_hash_descendant_anywhere_when_any_direct_argument_is_send() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(([{a: 1}]), bar(1))\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens when any direct send arg is present and the call contains a braced hash descendant"
        );
    }

    #[test]
    fn omit_accepts_parens_in_match_pattern() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"execute(query) in {elapsed:, sql_count:}\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens in match pattern");
    }

    #[test]
    fn omit_flags_index_assignment_value_calls() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"session[:x] = foo(bar)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag assigned values under []= parent calls"
        );
    }

    #[test]
    fn omit_flags_setter_assignment_value_calls() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"self.value = foo(bar)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag assigned values under setter parent calls"
        );
    }

    #[test]
    fn omit_flags_grouped_inner_call_argument() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo((bar(1)))\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            2,
            "Should flag both the outer grouped call and the grouped inner argument call"
        );
    }

    #[test]
    fn omit_flags_grouped_assignment_condition_value_call() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"if (server = response.take(1))\nend\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag grouped assignment-condition value calls"
        );
    }

    #[test]
    fn omit_accepts_operator_methods() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"data.[](value)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(diags.is_empty(), "Should allow parens on operator method");
    }

    #[test]
    fn omit_flags_last_in_chain() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo().bar(3).wait(4)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag only the last parenthesized call in chain"
        );
    }

    #[test]
    fn omit_flags_do_end_block() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(:arg) do\n  bar\nend\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(diags.len(), 1, "Should flag parens in do-end block call");
    }

    #[test]
    fn omit_accepts_parens_in_single_line_inheritance() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"class Point < Struct.new(:x, :y); end\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens in single-line inheritance"
        );
    }

    #[test]
    fn omit_accepts_forwarded_args() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def delegated_call(...)\n  @proxy.call(...)\nend\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens for forwarded arguments"
        );
    }

    #[test]
    fn allowed_methods_exempts() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "AllowedMethods".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("custom_log".into())]),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo.custom_log 'msg'\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag method in AllowedMethods list"
        );
    }

    #[test]
    fn allowed_patterns_exempts() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "AllowedPatterns".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("^assert".into())]),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo.assert_equal 'x', y\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag method matching AllowedPatterns"
        );
    }

    #[test]
    fn ignore_macros_false_flags_receiverless() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([("IgnoreMacros".into(), serde_yml::Value::Bool(false))]),
            ..CopConfig::default()
        };
        let source = b"custom_macro :arg\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag receiverless macro with IgnoreMacros:false"
        );
    }

    #[test]
    fn ignore_macros_skips_receiverless() {
        let source = b"custom_macro :arg\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(
            diags.is_empty(),
            "Should skip receiverless macro with IgnoreMacros:true"
        );
    }

    #[test]
    fn included_macros_forces_check() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "IncludedMacros".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("custom_macro".into())]),
            )]),
            ..CopConfig::default()
        };
        let source = b"custom_macro :arg\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag macro in IncludedMacros despite IgnoreMacros:true"
        );
    }

    #[test]
    fn included_macro_patterns_forces_check() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "IncludedMacroPatterns".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("^validate".into())]),
            )]),
            ..CopConfig::default()
        };
        let source = b"validates_presence :name\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag macro matching IncludedMacroPatterns"
        );
    }

    #[test]
    fn omit_allow_multiline_call() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("omit_parentheses".into()),
                ),
                (
                    "AllowParenthesesInMultilineCall".into(),
                    serde_yml::Value::Bool(true),
                ),
            ]),
            ..CopConfig::default()
        };
        let source = b"foo.bar(\n  1\n)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens in multiline call with AllowParenthesesInMultilineCall"
        );
    }

    #[test]
    fn omit_allow_chaining() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("omit_parentheses".into()),
                ),
                (
                    "AllowParenthesesInChaining".into(),
                    serde_yml::Value::Bool(true),
                ),
            ]),
            ..CopConfig::default()
        };
        let source = b"foo().bar(3).quux.wait(4)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens when chaining with previous parens"
        );
    }

    #[test]
    fn omit_allow_camel_case() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("omit_parentheses".into()),
                ),
                (
                    "AllowParenthesesInCamelCaseMethod".into(),
                    serde_yml::Value::Bool(true),
                ),
            ]),
            ..CopConfig::default()
        };
        let source = b"Array(1)\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens on CamelCase method with AllowParenthesesInCamelCaseMethod"
        );
    }

    #[test]
    fn omit_allow_string_interpolation() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("omit_parentheses".into()),
                ),
                (
                    "AllowParenthesesInStringInterpolation".into(),
                    serde_yml::Value::Bool(true),
                ),
            ]),
            ..CopConfig::default()
        };
        let source = b"x = \"#{foo.bar(1)}\"\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "Should allow parens inside string interpolation"
        );
    }

    #[test]
    fn yield_with_args_flagged() {
        let source = b"def foo\n  yield item\nend\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert_eq!(diags.len(), 1, "yield with args should be flagged");
    }

    #[test]
    fn yield_with_parens_ok() {
        let source = b"def foo\n  yield(item)\nend\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(diags.is_empty(), "yield with parens should be ok");
    }

    #[test]
    fn yield_no_args_ok() {
        let source = b"def foo\n  yield\nend\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(diags.is_empty(), "yield with no args should be ok");
    }

    #[test]
    fn yield_at_top_level_is_macro() {
        // yield at top level is macro scope — skipped with IgnoreMacros: true
        let source = b"yield item\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(
            diags.is_empty(),
            "yield at top level should be treated as macro"
        );
    }

    #[test]
    fn omit_yield_flags_parens() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def foo\n  yield(item)\nend\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag yield parens with omit_parentheses"
        );
    }

    #[test]
    fn omit_yield_no_parens_ok() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("omit_parentheses".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def foo\n  yield item\nend\n";
        let diags = run_cop_full_with_config(&MethodCallWithArgsParentheses, source, config);
        assert!(
            diags.is_empty(),
            "yield without parens should be ok in omit_parentheses"
        );
    }

    #[test]
    fn lambda_in_class_body_preserves_macro_scope() {
        let source = b"class C\n  subject { -> { get :index } }\nend\n";
        let diags = run_cop_full(&MethodCallWithArgsParentheses, source);
        assert!(
            diags.is_empty(),
            "Receiverless call inside lambda in class body should be treated as macro"
        );
    }
}
