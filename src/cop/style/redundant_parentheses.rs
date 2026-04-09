use ruby_prism::Visit;

use crate::cop::shared::method_identifier_predicates;
use crate::cop::shared::predicate_operator_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/RedundantParentheses checks for redundant parentheses around expressions.
///
/// ## Investigation findings (2026-03-08)
///
/// ### FP root causes fixed:
/// - `(-2)**2`: negative numeric literal as exponentiation base — parens required to
///   distinguish from `-(2**2)`. Fixed by checking `raised_to_power_negative_numeric`.
/// - `-(1.foo)`, `+(1.foo)`: unary minus/plus applied to method chain starting with
///   integer literal — removing parens would parse as `(-1).foo`. Fixed by checking
///   `call_chain_starts_with_int`.
/// - `(not x)`, `(a while b)`, `(a until b)`: keyword expressions that RuboCop
///   considers plausible. Fixed by skipping NotNode, WhileNode, UntilNode inner nodes.
/// - Comparison `x && (y == z)`: was using stack depth heuristic. Fixed to only flag
///   when parent is truly nil (no real parent node).
/// - `super ({...})`, `yield ({...})`: hash as first arg to unparenthesized super/yield
///   needs parens to avoid parsing as block. Added SuperNode/YieldNode to
///   like_method_argument_parentheses check.
/// - `super (42)` multiline, `yield (42)` multiline: multiline control flow also
///   applies to super/yield. Added SuperNode/YieldNode to multiline check.
/// - Multiple expressions `(foo; bar)` in non-begin parent: RuboCop only flags in
///   begin/def/block contexts.
/// - `var = (foo or bar)`: keyword-form logical in assignment context is allowed.
/// - Various keyword-adjacent parens: rescue(), when(), else(), while-post, until-post.
///
/// ### FN root causes fixed:
/// - Unary operations `(!x)`, `(~x)`, `(-x)`, `(+x)`: added unary operation detection.
/// - Lambda/proc with braces: `(-> { x })`, `(lambda { x })`, `(proc { x })`.
/// - Keywords: `(defined?(:A))`, `(yield)`, `(yield())`, `(yield(1,2))`, `(super)`,
///   `(super())`, `(super(1,2))` — added keyword_with_redundant_parentheses detection.
/// - `===` comparison: added to is_comparison.
/// - Method argument: `x.y((z))` — added argument_of_parenthesized_method_call.
/// - One-line rescue: `(foo rescue bar)` at top level.
/// - `return (42)`, `return (foo + bar)`: return/next/break with space before paren
///   and non-multiline content should still flag the inner expression.
///
/// ## Investigation findings (2026-03-15)
///
/// ### FP root causes fixed:
/// - **Chained receiver as method argument (major, ~thousands of FPs):** `(expr).method(args)`
///   was flagged as "a method argument" because the paren node's parent Call was parenthesized,
///   but the paren is the *receiver*, not an argument. RuboCop checks
///   `parent.receiver != begin_node`. Fixed by adding `is_chained` check in
///   `check_argument_of_parenthesized_call` — if `)` is followed by `.`/`&.`, it's a receiver.
/// - **`[]` calls treated as parenthesized:** `call_parenthesized` was set from
///   `opening_loc().is_some()` which is true for `[` (bracket calls). RuboCop's
///   `parenthesized?` only matches `(`. Fixed by checking `opening_loc` is specifically `"("`.
/// - **Hash literal first arg of unparenthesized call:** `x ({y: 1}), z` — parens needed to
///   prevent `{` from being parsed as a block. RuboCop's `first_arg_begins_with_hash_literal?`
///   catches this. Added simplified equivalent: skip when inner begins with hash literal and
///   there's an unparenthesized Call ancestor.
///
/// ## Investigation findings (2026-03-17)
///
/// ### FP root causes fixed:
/// - **Default parameter assignments (~100+ FPs):** `def method(value = (not_set = true))` —
///   parenthesized assignment in default parameter values is syntactically required. Was flagged
///   because OptionalParameterNode mapped to ParentKind::Other, which the assignment check
///   treats as begin_type. Fixed by adding ParentKind::Parameter set on
///   visit_optional_parameter_node / visit_optional_keyword_parameter_node.
/// - **`class << (RANDOM = Random.new)`:** assignment in singleton class expression receiver.
///   Fixed by adding ParentKind::SingletonClass.
/// - **`def (@matcher = BasicObject.new).===(obj)`:** assignment in def receiver expression.
///   Fixed by adding ParentKind::Def.
///
/// ## Investigation findings (2026-03-18)
///
/// ### FP root causes fixed:
/// - **`def (@obj).method` singleton method receiver (~16+ FPs from hexapdf):** parens around
///   the receiver in singleton method definitions are always required. Added early return when
///   parent is `ParentKind::Def`.
/// - **`&(l = -> {})` block argument with assignment (~25+ FPs from jruby):** assignment inside
///   `&()` block pass is required. Added `ParentKind::BlockArgument` tracking so assignment
///   check doesn't flag it (block_pass is not nil/begin_type in RuboCop).
/// - **`(-8.0) ** expr` with space before `**` (~3 FPs):** `is_raised_to_power_negative_numeric`
///   wasn't skipping whitespace between `)` and `**`. Fixed to skip spaces.
/// - **`(!(found = find_file(exe)))` unary around paren+assignment:** the unary check now
///   recognizes when the base receiver (after unwrapping nested unary ops) is a
///   ParenthesesNode and skips, since the outer parens wrap a necessary sub-expression.
/// - **`(t = expr) rescue nil` inline rescue with assignment:** added `ParentKind::RescueModifier`
///   tracking. Rescue modifier is not nil/begin_type in RuboCop.
/// - **`a, *b = (e = [1,2,3])` multiple assignment RHS:** added `ParentKind::MultipleAssignment`
///   tracking for `MultiWriteNode`.
/// - **Pattern matching `in`/`=>` expressions in non-top-level contexts:** `(Value(1) in [1])`,
///   `(a => b)` — parens around pattern matching should not be flagged when inside method args,
///   boolean operators (&&/||), assignments, or endless method definitions. Added
///   `MatchPredicateNode` and `MatchRequiredNode` recognition.
///
/// ### FN root causes fixed:
/// - **Range in method argument:** `x.y((a..b))` — the early return for ranges skipped
///   the argument-of-parenthesized-call check. Fixed by checking method arg first for ranges.
/// - **Interpolated expressions:** `"#{(foo)}"` — added `ParentKind::Interpolation` tracking
///   for `EmbeddedStatementsNode` and detection of redundant parens inside string interpolation.
/// - **Pattern matching at top level:** `(expression in pattern)`, `(expression => pattern)` —
///   added detection for `MatchPredicateNode`/`MatchRequiredNode` with appropriate exemptions.
///
/// ## Investigation findings (2026-04-01)
///
/// ### FN root causes fixed:
/// - **Chained method calls (~380 FN resolved):** `(foo.bar).to_s`, `(expr.method(args)).chain`,
///   `!(@groups.include?(g))`, `(select {...})[key]` — the `is_chained`/`is_receiver` check
///   in `check_method_call` incorrectly blocked method call detection when parens were followed
///   by `.` or used as the receiver of a parent call. RuboCop's `check_send` does NOT check
///   `chained?` for method calls — only for logical/comparison/unary expressions. Removed the
///   `is_chained || is_receiver` guard from `check_method_call` (kept in `check_unary`).
///   Also added `Super`/`Yield` to the singular parent check for `method_call_with_redundant_parentheses?`.
/// - **Assignment in method argument:** `foo.include?((port = get_port))` — the assignment check
///   returned early even when not flagging, preventing the method argument check from catching
///   assignments inside parenthesized calls. Fixed by only falling through when parent is a
///   parenthesized call, avoiding FPs on assignments in boolean context like `(x = y) && z`.
///
/// ## Investigation findings (2026-04-04)
///
/// ### FN root causes fixed:
/// - **Single-child parent contexts for operator sends:** `[(call + arg)]`, `((1 << 128)).to_s`,
///   and nested forms like `(((c & mask)) << 10)` were still missed because the
///   `method_call_with_redundant_parentheses?` approximation only treated top-level control-flow
///   parents as "singular". RuboCop also flags these when the immediate Prism container has a
///   single child, so `StatementsNode` and `ArrayNode` now track that shape and feed the
///   singular parent check.
/// - **Parenthesized block calls as parenthesized-call arguments:** `match(event, (on Finished do
///   ... end))` was skipped because Prism exposes it as a `CallNode` with an attached block,
///   while RuboCop effectively treats that argument like a block expression for
///   `argument_of_parenthesized_method_call?`. The method-argument exemption now only applies to
///   unparenthesized calls that truly need parens to preserve call parsing, not block-attached
///   calls that RuboCop still flags.
/// - **Backtick command literals:** Prism parses `` `cmd` `` and `` `cmd #{x}` `` as
///   `XStringNode` / `InterpolatedXStringNode`, not string nodes. Missing those literal node kinds
///   dropped offenses like ``(`curl ...`).split(" ")`` and bare parenthesized command literals in
///   blocks. They now classify as literals like RuboCop does.
/// - **Receiver parens on unparenthesized single-arg calls:** `(Array(...)).include? foo` was
///   incorrectly treated like a "method argument parentheses required" case because the
///   like-method-argument approximation only checked the parent call shape. It now also verifies
///   the parens are not the parent call's receiver, preserving true argument cases like
///   `x.y((z))`.
///
/// ### FP root causes fixed:
/// - **Command literals as the RHS of `=~`:** `/Version/m =~ (`convert -version`)` stays accepted
///   by RuboCop even though standalone parenthesized xstrings are redundant. After teaching the cop
///   that Prism xstrings are literals, it now keeps a narrow exemption when the parenthesized
///   xstring is the argument to a match operator call.
///
/// ## Investigation findings (2026-04-06)
///
/// ### FN root causes fixed:
/// - **Assignment in begin-like statement containers:** Prism exposes method/block/BEGIN bodies and
///   nested `if((var = ...))` conditions through `StatementsNode` wrappers, so the previous
///   grandparent/conditional heuristic skipped real `begin_type?` cases. Assignment parens now
///   flag whenever the immediate parent is a non-assignment `StatementsNode`, which restores
///   RuboCop parity for standalone body statements without re-flagging assignments used as the
///   RHS of another assignment or inside endless method-definition bodies.
/// - **Empty-body `while` conditions with operator calls:** RuboCop treats `while (call > 0); end`
///   like a singular-parent context, but Prism `WhileNode` needed an explicit empty-body check.
///   Empty-body `while` nodes now participate in the singular-parent method-call rule, while
///   normal loop bodies remain accepted.
///
/// ### FP root causes fixed (2026-04-07):
/// - **Assignment in block body:** `loop { (i = 1) }`, `[1].each { |x| (i = x) }` were incorrectly
///   flagged as "redundant parentheses around an assignment" because the parent stack showed the
///   assignment's parent as a `StatementsNode` (kind=Other) rather than recognizing it was inside
///   a block body. RuboCop doesn't flag these. Fixed by adding `is_parent_statements_block_body`
///   check: when the grandparent of the assignment's parent statements is a CallNode or BlockNode,
///   the assignment is inside a block and should not be flagged.
///
/// ## Investigation findings (2026-04-08)
///
/// ### FN root causes fixed:
/// - **Setter/index-write sends in begin-like contexts:** Prism exposes `self[field] = value` and
///   `self.foo = value` as `CallNode`s with `equal_loc`, so they were skipped by the assignment
///   path entirely. RuboCop treats these like assignments at top level / begin-like statement
///   parents, but still falls back to send-based handling in contexts like `return (...)`,
///   `if (...)`, and `foo((...))`. Fixed by recognizing assignment-like calls separately and only
///   short-circuiting them in the same contexts RuboCop exempts (single-statement block bodies,
///   endless-def bodies).
/// - **Ternary assignment branches stay exempt:** once assignment-like calls started sharing the
///   assignment path, ternary branches like `(cond ? (x |= y) : z)` were incorrectly reported.
///   RuboCop keeps assignment parens anywhere inside ternary conditions/branches, so the
///   assignment fast-path now bails out when a ternary ancestor is present.
/// - **Single-statement conditional bodies stay exempt:** modifier and block-style conditional
///   bodies like `(count += 1) unless skip` and `if cond; (count += 1); end` are accepted by
///   RuboCop even though the immediate Prism parent is still a `StatementsNode`. The assignment
///   path now recognizes one-statement `if`/`unless`/`while`/`until`/`case` bodies and skips
///   them instead of treating every statements wrapper as begin-like. Multi-statement bodies still
///   flag because RuboCop treats those like `begin_type?` containers.
/// - **Modifier-if rescue bodies still flag:** `after { (r.quit rescue nil) if defined?(r) }`
///   is a body expression, not a conditional predicate. The one-line-rescue exemption now checks
///   whether the parens are actually inside a conditional predicate range instead of skipping any
///   nearby `if`/`while`/`until` ancestor.
/// - **Receiver parens after `until(`/`while(` still flag when chained:** `until($stdin.gets).include?`
///   uses keyword-adjacent parens, but they wrap the receiver of a chained call, not the whole
///   loop predicate. The keyword-adjacency fast path now stays limited to non-receiver cases like
///   `end until(bar)`.
/// - **Unary `+/-` integer-chain exemption was too broad:** the `-(1.foo)` safeguard only applies
///   to unary `+@`/`-@` parents. It now tracks true unary parents so binary operators like
///   `1-(1.quo(ii))` still report redundant parens.
/// - **Hash-first-argument exemption was too broad:** `first_arg_begins_with_hash_literal` used any
///   unparenthesized call ancestor, which incorrectly exempted later arguments like
///   `foo a, (({ y: 1 }.merge(z)))`. RuboCop only allows this when the parenthesized expression
///   itself, or a receiver chain rooted at it, is the first argument of an unparenthesized call.
///   Fixed by tracking first-argument start offsets and only applying the exemption to true
///   first-argument chains.
///
/// ### FP root causes fixed:
/// - **Paren descendants inside a hash first arg were still flagged:** RuboCop treats
///   `foo :plain => ({...}.to_json)` and `Contract ({...}) => Num` like other
///   hash-first-argument cases because the containing hash literal is the first argument of an
///   unparenthesized call. Prism inserts `AssocNode`/hash ancestors between the parens and the
///   call, so the previous offset walk only saw direct call/receiver chains and missed these
///   nested cases. The exemption now climbs through pair/hash ancestors before checking the
///   unparenthesized first-argument call boundary, while still flagging standalone hashes like
///   `x = { plain: ({...}.to_json) }`.
///
/// ## Investigation findings (2026-04-09)
///
/// ### FN root causes fixed:
/// - **Rescue inside assignment in conditional predicate:** `if (var = (expr rescue nil))` — the
///   inner `(expr rescue nil)` was not flagged because `is_in_conditional_predicate` used a
///   containment check (any parens within the predicate range). RuboCop checks exact identity
///   (`parent.condition == begin_node`), so only the direct predicate is exempt. Changed to
///   exact range match so nested rescue-in-assignment is correctly flagged.
///
/// ### FP root causes fixed:
/// - **Assignment in single-statement def body (~6+ FPs from ebnf):** `def foo; (@var ||= {}); end`
///   — Prism always wraps bodies in StatementsNode, but Parser AST only adds a `begin` wrapper
///   for multi-statement bodies. Single-statement non-paren bodies map to the containing node
///   directly, which is NOT begin_type. The `begin_like_parent` condition now correctly requires
///   either parentheses body, multi-statement body, or top-level context.
/// - **Prefix `!` (prefix_not) exempt from unary check (~7+ FPs from norikra):** RuboCop's
///   `method_call_with_redundant_parentheses?` returns false for `prefix_not?` nodes. All
///   `(!expr)` patterns are exempt: `suspect_unary?` doesn't unwrap through `!`, and then
///   `prefix_not?` causes an early return. Added the same exemption.
/// - **Method call with unparenthesized block argument:** `(method_args.map &:to_json).join(',')`
///   — Prism puts block arguments in `block()` not `arguments()`, so the cop didn't count them
///   as "has args". Removing the outer parens would change parsing. Now counts block_argument
///   nodes for the singular-parent check.
pub struct RedundantParentheses;

impl Cop for RedundantParentheses {
    fn name(&self) -> &'static str {
        "Style/RedundantParentheses"
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let mut visitor = RedundantParensVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            parent_stack: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ParentKind {
    And,
    Or,
    Call,
    Hash,
    Splat,
    KeywordSplat,
    Return,
    Next,
    Break,
    Ternary,
    Range,
    Super,
    Yield,
    If,
    While,
    Until,
    Case,
    Array,
    Pair,
    Parameter,
    SingletonClass,
    Def,
    Block,
    BlockArgument,
    RescueModifier,
    MultipleAssignment,
    Interpolation,
    Other,
}

struct ParentInfo {
    kind: ParentKind,
    multiline: bool,
    single_child: bool,
    is_statements_node: bool,
    is_parentheses_body: bool,
    is_parentheses_node: bool,
    call_parenthesized: bool,
    call_arg_count: usize,
    is_operator: bool,
    is_unary_plus_minus: bool,
    is_match_operator: bool,
    is_endless_def: bool,
    is_assignment_parent: bool,
    /// The start offset of this parent node.
    /// Used to walk receiver chains for first-argument hash-literal exemptions.
    node_start_offset: usize,
    /// For Call parents, the start offset of the receiver node (if any).
    /// Used to implement RuboCop's `begin_node.chained?` check.
    call_receiver_start_offset: Option<usize>,
    /// For Call parents, the start offset of the first argument (if any).
    /// Used to implement RuboCop's `first_argument?` check for hash-literal exemptions.
    call_first_arg_start_offset: Option<usize>,
    /// For Call parents, whether the first argument is a ParenthesesNode.
    /// Used to implement RuboCop's `like_method_argument_parentheses?` which
    /// checks `node.first_argument.begin_type?`.
    call_first_arg_is_begin: bool,
    /// For StatementsNode parents, how many children the StatementsNode has.
    /// Used to distinguish single-statement vs multi-statement block bodies.
    statements_child_count: usize,
    /// For RescueModifier parents, the start offset of the rescue_expression
    /// (right side). RuboCop only exempts parens in the rescue body (resbody),
    /// which corresponds to the rescue_expression in Prism. The expression
    /// (left side) is still flagged.
    rescue_expression_start_offset: Option<usize>,
    /// For conditional parents, the source range of the predicate expression.
    /// Used to distinguish parens in the condition from parens in the body.
    conditional_predicate_range: Option<(usize, usize)>,
}

struct RedundantParensVisitor<'a> {
    cop: &'a RedundantParentheses,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    parent_stack: Vec<ParentInfo>,
}

impl RedundantParensVisitor<'_> {
    fn check_parens(&mut self, node: &ruby_prism::ParenthesesNode<'_>) {
        let body = match node.body() {
            Some(b) => b,
            None => return, // empty parens ()
        };

        let inner_nodes: Vec<ruby_prism::Node<'_>> = if let Some(stmts) = body.as_statements_node()
        {
            stmts.body().iter().collect()
        } else {
            vec![body]
        };

        // parent_stack.last() is the ParenthesesNode's own entry (pushed by
        // visit_branch_node_enter). The actual parent is one level up.
        let parent = if self.parent_stack.len() >= 2 {
            Some(&self.parent_stack[self.parent_stack.len() - 2])
        } else {
            None
        };
        let is_receiver = self.is_receiver_of_parent_call(node, parent);

        // Multiple expressions like (foo; bar) — skip entirely.
        // RuboCop only flags these in begin/def/block contexts, but distinguishing
        // those from assignment/call/etc contexts in our parent stack is fragile.
        // Skipping avoids FPs on patterns like x = (foo; bar).
        if inner_nodes.len() != 1 {
            return;
        }

        let inner = &inner_nodes[0];

        // like_method_argument_parentheses? — applies to send, super, yield
        // RuboCop checks: parent has one arg, not parenthesized, not operator,
        // and first arg is begin_type?. When true, ALL begin nodes under this
        // parent are skipped (including the receiver).
        if let Some(p) = parent {
            let is_like_method_arg = match p.kind {
                ParentKind::Call => {
                    !p.call_parenthesized
                        && !p.is_operator
                        && p.call_arg_count == 1
                        && p.call_first_arg_is_begin
                }
                ParentKind::Super | ParentKind::Yield => {
                    !p.call_parenthesized && p.call_arg_count == 1
                }
                _ => false,
            };
            if is_like_method_arg {
                return;
            }
        }

        // multiline_control_flow_statements? — applies to return, next, break, super, yield
        if let Some(p) = parent {
            if matches!(
                p.kind,
                ParentKind::Return
                    | ParentKind::Next
                    | ParentKind::Break
                    | ParentKind::Super
                    | ParentKind::Yield
            ) && p.multiline
            {
                return;
            }
        }

        // allowed_ancestor? — don't flag `break(value)`, `return(value)`, `next(value)`,
        // `super(value)`, `yield(value)`, `rescue(err)`, `when(val)`, `else(val)`
        // when the keyword is directly adjacent to the open paren (no space).
        if let Some(p) = parent {
            if matches!(
                p.kind,
                ParentKind::Return
                    | ParentKind::Next
                    | ParentKind::Break
                    | ParentKind::Super
                    | ParentKind::Yield
            ) {
                let open_offset = node.location().start_offset();
                if open_offset > 0 {
                    let before = self.source.content[open_offset - 1];
                    if before.is_ascii_alphabetic() || before == b'?' {
                        return;
                    }
                }
            }
        }

        // Parens touching a preceding keyword (like `else(1)` or `(1)end`)
        // Check if a keyword character immediately precedes the open paren
        // This catches patterns like `if x; y else(1) end`
        {
            let open_offset = node.location().start_offset();
            if open_offset > 0 {
                let before = self.source.content[open_offset - 1];
                if before.is_ascii_alphabetic() {
                    // Check if we're right after a keyword like 'else', 'do', etc.
                    // Only skip if not in return/next/break/super/yield (those are handled above).
                    // Receiver parens like `until(call).chain` are still redundant.
                    if !is_receiver
                        && parent
                            .map(|p| {
                                !matches!(
                                    p.kind,
                                    ParentKind::Return
                                        | ParentKind::Next
                                        | ParentKind::Break
                                        | ParentKind::Super
                                        | ParentKind::Yield
                                )
                            })
                            .unwrap_or(true)
                    {
                        return;
                    }
                }
            }
            // Check if close paren immediately precedes a keyword
            let close_offset = node.location().end_offset();
            if close_offset < self.source.content.len() {
                let after = self.source.content[close_offset];
                if after.is_ascii_alphabetic() {
                    return;
                }
            }
        }

        // range parent
        if let Some(p) = parent {
            if matches!(p.kind, ParentKind::Range) {
                return;
            }
        }

        // RuboCop's rescue? check: parens inside the rescue body (right side) are
        // always allowed. `x rescue (y || z)` — the `(y || z)` is exempt.
        // RuboCop checks `^resbody ^^resbody` which matches children/grandchildren
        // of the rescue body node. The expression (left side) is NOT exempt.
        if parent.is_some_and(|p| {
            matches!(p.kind, ParentKind::RescueModifier)
                && p.rescue_expression_start_offset
                    .is_some_and(|off| node.location().start_offset() >= off)
        }) {
            return;
        }

        // Assignment — RuboCop flags `(assignment)` when the immediate parent is nil or
        // begin_type?. In Prism the comparable container is a plain StatementsNode, but we
        // must not treat an outer assignment node the same way: `index = (state[:index] = ...)`
        // keeps its parens in RuboCop. We therefore only flag StatementsNode-style parents
        // that are not themselves assignment nodes.
        //
        // NOTE: When not flagging as assignment, only fall through to the
        // method argument check when inside a parenthesized call. This catches
        // `foo.include?((port = get_port))` without causing FPs on assignments
        // in boolean context like `(x = y) && z`.
        let assignment_call = is_assignment_call(inner);
        if is_assignment(inner) || assignment_call {
            if self.has_ternary_ancestor() || self.is_parent_statements_conditional_body() {
                return;
            }

            let should_flag = match parent {
                None => true,
                Some(p) => {
                    // In Parser AST, single-statement bodies (def, if, etc.) do NOT
                    // get a `begin` wrapper — only multi-statement bodies do.
                    // Parentheses nodes inside parens always map to begin_type.
                    // Top-level (program body) is always begin-like.
                    let begin_like_parent = p.is_statements_node
                        && (p.is_parentheses_body
                            || p.statements_child_count > 1
                            || self.parent_stack.len() <= 2);
                    begin_like_parent
                        && !p.is_assignment_parent
                        && !self.is_endless_def_body_parent()
                        && !self.is_parent_statements_block_body()
                }
            };
            if should_flag {
                self.add_offense(node, "an assignment");
                return;
            }
            // Setter/index-write sends still participate in RuboCop's send-based checks
            // in contexts like `return (...)`, `if (...)`, and `foo((...))`, but block
            // bodies and endless defs keep their parens.
            if assignment_call
                && (self.is_parent_statements_block_body() || self.is_endless_def_body_parent())
            {
                return;
            }
            // Non-call assignments can only fall through when they are a method argument.
            if !assignment_call {
                let is_method_arg_candidate = parent
                    .is_some_and(|p| matches!(p.kind, ParentKind::Call) && p.call_parenthesized);
                if !is_method_arg_candidate {
                    return;
                }
            }
        }

        // Range literals — skip unless it's a method argument of a parenthesized call
        // (RuboCop flags x.y((a..b)) as "a method argument") or double-parens ((1..42)).
        // The method argument check is handled below in check_argument_of_parenthesized_call.
        if inner.as_range_node().is_some() {
            // Check if this is an argument of a parenthesized method call first
            if let Some(msg) = self.check_argument_of_parenthesized_call(node, inner, parent) {
                self.add_offense(node, msg);
                return;
            }
            return;
        }

        // Skip `not` keyword expressions — (not x) is plausible
        // Prism represents `not x` as CallNode with name `!` but message_loc `not`
        if inner.as_call_node().is_some_and(|c| {
            c.name().as_slice() == b"!" && c.message_loc().is_some_and(|m| m.as_slice() == b"not")
        }) {
            return;
        }

        // Skip while/until modifier expressions — (a while b), (a until b) are plausible
        if inner.as_while_node().is_some() || inner.as_until_node().is_some() {
            return;
        }

        // One-line rescue — (foo rescue bar) is flagged at top level/begin but not
        // in certain contexts (ternary, conditional, array, hash, method arg)
        if inner.as_rescue_modifier_node().is_some() {
            if let Some(msg) = self.check_one_line_rescue(node, parent) {
                self.add_offense(node, msg);
            }
            return;
        }

        // Keyword detection: defined?, yield, super, return, next, break
        if let Some(msg) = self.check_keyword_with_redundant_parens(inner) {
            self.add_offense(node, msg);
            return;
        }

        // Lambda/proc with braces — (-> { x }), (lambda { x }), (proc { x })
        if is_lambda_or_proc_with_braces(inner) {
            self.add_offense(node, "an expression");
            return;
        }

        // One-line pattern matching: (expr in pattern), (expr => pattern)
        if let Some(msg) = self.check_pattern_matching(inner, parent) {
            self.add_offense(node, msg);
            return;
        }

        // Interpolation: "#{(foo)}" — parens inside string interpolation are redundant
        if self.is_interpolation(parent) {
            self.add_offense(node, "an interpolated expression");
            return;
        }

        // Check if this is an argument of a parenthesized method call
        // e.g., x.y((z)), x.y((z + w)), x.y(a, (b))
        if let Some(msg) = self.check_argument_of_parenthesized_call(node, inner, parent) {
            self.add_offense(node, msg);
            return;
        }

        // first_arg_begins_with_hash_literal? — when the inner expression is (or starts
        // with) a hash literal, and the paren is the first argument of an unparenthesized
        // method call, the parens are needed to prevent `{` from being parsed as a block.
        if self.first_arg_begins_with_hash_literal(node, inner, parent) {
            return;
        }

        // def (expr).method — parens around singleton method receiver are always required.
        // RuboCop doesn't flag these because `def` receivers don't produce `on_begin` events.
        if parent.is_some_and(|p| matches!(p.kind, ParentKind::Def)) {
            return;
        }

        // RuboCop accepts parenthesized command literals on the RHS of `=~`.
        // Example: `/Version/m =~ (`convert -version`)`
        if is_xstring(inner)
            && parent.is_some_and(|p| matches!(p.kind, ParentKind::Call) && p.is_match_operator)
        {
            return;
        }

        if let Some(msg) = classify_simple(inner) {
            // Check for negative numeric in exponentiation base: (-2)**2 is plausible
            if msg == "a literal"
                && is_raised_to_power_negative_numeric(inner, node, &self.source.content)
            {
                return;
            }
            self.add_offense(node, msg);
            return;
        }

        // RuboCop's `begin_node.chained?` — if the ParenthesesNode is the receiver
        // of a parent Call (including operators and unary calls), skip logical,
        // comparison, and method-call/unary checks.
        // Logical expression
        if inner.as_and_node().is_some() || inner.as_or_node().is_some() {
            if let Some(msg) = check_logical(&self.source.content, node, inner, parent, is_receiver)
            {
                self.add_offense(node, msg);
                return;
            }
        }

        // Comparison expression — only flagged when parent is nil (truly top-level).
        // RuboCop checks `begin_node.parent.nil?`.
        // In Prism, ProgramNode calls visit_statements_node directly (no push),
        // so the stack is [Program, ParenthesesNode] for top-level expressions.
        // len <= 2 approximates "no parent" (program root only).
        if is_comparison(inner)
            && !is_receiver
            && !is_chained(&self.source.content, node)
            && self.parent_stack.len() <= 2
            && parent.is_none_or(|p| matches!(p.kind, ParentKind::Other))
        {
            self.add_offense(node, "a comparison expression");
            return;
        }

        // Method call (includes unary operations)
        if inner.as_call_node().is_some() {
            if let Some(msg) =
                check_method_call(&self.source.content, node, inner, parent, is_receiver)
            {
                self.add_offense(node, msg);
            }
        }
    }

    fn add_offense(&mut self, node: &ruby_prism::ParenthesesNode<'_>, msg: &str) {
        let loc = node.location();
        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            format!("Don't use parentheses around {}.", msg),
        ));
    }

    /// Check if a nearby ancestor is a ternary, looking through intermediate
    /// wrapper nodes (StatementsNode, ElseNode) that Prism inserts.
    fn has_ternary_ancestor(&self) -> bool {
        if self.parent_stack.len() < 2 {
            return false;
        }
        // Start at len-2 (skip the ParenthesesNode's own entry)
        for i in (0..self.parent_stack.len() - 1).rev() {
            match self.parent_stack[i].kind {
                ParentKind::Ternary => return true,
                ParentKind::Other => continue,
                _ => return false,
            }
        }
        false
    }

    fn is_endless_def_body_parent(&self) -> bool {
        if self.parent_stack.len() < 3 {
            return false;
        }

        let grandparent = &self.parent_stack[self.parent_stack.len() - 3];
        matches!(grandparent.kind, ParentKind::Def) && grandparent.is_endless_def
    }

    /// Check if the parent (a StatementsNode) is the body of a SINGLE-statement
    /// block. In RuboCop AST, a single-statement block body has the begin node's
    /// parent as the `:block` node (not begin_type), so `assignment?` checks fail.
    /// A multi-statement block body wraps children in a `:begin` node, so the
    /// begin node's parent IS begin_type and the assignment IS flagged.
    ///
    /// We replicate this by checking: the grandparent is a Block/Call, AND the
    /// parent StatementsNode has exactly one child (single-statement block).
    fn is_parent_statements_block_body(&self) -> bool {
        // parent_stack structure for `(assignment)` inside a block:
        // [..., CallNode/BlockEntry, OuterStatements, ParenthesesNode, InnerStatements]
        // grandparent = parent_stack[len - 3]
        if self.parent_stack.len() < 3 {
            return false;
        }
        let parent = &self.parent_stack[self.parent_stack.len() - 2];
        let grandparent = &self.parent_stack[self.parent_stack.len() - 3];
        // Only suppress for single-statement block bodies. Multi-statement block
        // bodies correspond to RuboCop's begin wrapper where assignments ARE flagged.
        matches!(grandparent.kind, ParentKind::Call | ParentKind::Block)
            && parent.statements_child_count == 1
    }

    /// Check if the parent (a StatementsNode) is the SINGLE statement body of a
    /// conditional-like construct (`if`/`unless`, `while`/`until`, `case`).
    /// RuboCop keeps assignment parens in these bodies, including modifier
    /// forms like `(count += 1) unless skip`, but multi-statement bodies get a
    /// `begin` wrapper in Parser AST and are flagged.
    fn is_parent_statements_conditional_body(&self) -> bool {
        if self.parent_stack.len() < 3 {
            return false;
        }

        let parent_index = self.parent_stack.len() - 2;
        let parent = &self.parent_stack[parent_index];
        if !parent.is_statements_node
            || parent.is_parentheses_body
            || parent.statements_child_count != 1
        {
            return false;
        }

        for i in (0..=parent_index).rev() {
            let info = &self.parent_stack[i];
            match info.kind {
                ParentKind::Other => continue,
                ParentKind::If | ParentKind::While | ParentKind::Until | ParentKind::Case => {
                    return true;
                }
                _ => return false,
            }
        }

        false
    }

    /// RuboCop's first_arg_begins_with_hash_literal?: when the inner expression
    /// starts with a hash literal and the paren is a first argument of an
    /// unparenthesized method call, parens are needed to prevent `{` from being
    /// parsed as a block.
    fn first_arg_begins_with_hash_literal(
        &self,
        node: &ruby_prism::ParenthesesNode<'_>,
        inner: &ruby_prism::Node<'_>,
        _parent: Option<&ParentInfo>,
    ) -> bool {
        // Check if the inner expression is or starts with a hash literal
        if !self.inner_begins_with_hash(inner) {
            return false;
        }

        // RuboCop checks `first_argument?(node)` — the begin node (or an ancestor
        // in the receiver chain) must be a first argument of some method call.
        // If the parens are the receiver of a parent call (chained), they're not
        // directly a first argument. But they might be indirectly (e.g.,
        // `x ({y: 1}).merge(z), w` — parens are receiver of .merge, but .merge
        // is a first arg of x).
        //
        self.is_first_argument_of_unparenthesized_call_chain(node)
    }

    /// Walk the receiver chain of call nodes to find a hash literal at the root.
    fn inner_begins_with_hash(&self, node: &ruby_prism::Node<'_>) -> bool {
        if node.as_hash_node().is_some() {
            return true;
        }
        if let Some(call) = node.as_call_node() {
            if let Some(recv) = call.receiver() {
                return self.inner_begins_with_hash(&recv);
            }
        }
        false
    }

    /// RuboCop's first_argument? allows hash-first-argument parens when the begin
    /// node itself, or a receiver chain rooted at it, becomes the first argument
    /// of an unparenthesized call.
    fn is_first_argument_of_unparenthesized_call_chain(
        &self,
        node: &ruby_prism::ParenthesesNode<'_>,
    ) -> bool {
        let mut current_start = node.location().start_offset();

        for i in (0..self.parent_stack.len().saturating_sub(1)).rev() {
            let info = &self.parent_stack[i];
            match info.kind {
                ParentKind::Other => continue,
                // RuboCop's `first_argument?` recurses through ancestors, so a begin node
                // nested inside an assoc/hash still counts when that containing hash is
                // the first argument of an unparenthesized call.
                ParentKind::Pair | ParentKind::Hash => {
                    current_start = info.node_start_offset;
                }
                ParentKind::Call => {
                    if info
                        .call_receiver_start_offset
                        .is_some_and(|start| start == current_start)
                    {
                        current_start = info.node_start_offset;
                        continue;
                    }

                    return info
                        .call_first_arg_start_offset
                        .is_some_and(|start| start == current_start)
                        && !info.call_parenthesized;
                }
                _ => return false,
            }
        }

        false
    }

    /// Check if inner node is a keyword with redundant parentheses.
    /// Handles: defined?, yield, super, return, next, break
    fn check_keyword_with_redundant_parens(
        &self,
        inner: &ruby_prism::Node<'_>,
    ) -> Option<&'static str> {
        // defined?(expr) — keyword when parenthesized, but (defined? expr) is plausible
        if let Some(defined) = inner.as_defined_node() {
            // Only flag when defined? uses parenthesized form: defined?(:A)
            // Check if the source has `defined?(` (no space between ? and ()
            let loc = defined.location();
            let src = &self.source.content[loc.start_offset()..loc.end_offset()];
            // defined? with parenthesized arg: `defined?(:A)` — keyword
            // defined? with unparenthesized arg: `defined? :A` — plausible
            if src.len() > 8 && src[8] == b'(' {
                return Some("a keyword");
            }
            return None;
        }

        // yield — keyword
        if let Some(yield_node) = inner.as_yield_node() {
            let args = yield_node
                .arguments()
                .map(|a| a.arguments().len())
                .unwrap_or(0);
            let has_parens = yield_node.lparen_loc().is_some();
            if args == 0 || has_parens {
                return Some("a keyword");
            }
            // (yield 1, 2) — plausible
            return None;
        }

        // super — keyword
        if let Some(_super_node) = inner.as_super_node() {
            // SuperNode in Prism is `super(args)` or `super args`
            // Check if it has parenthesized args
            let loc = inner.location();
            let src = &self.source.content[loc.start_offset()..loc.end_offset()];
            // super() or super(1,2) — has parens after 'super'
            // super 1, 2 — no parens
            let after_keyword = &src[5..]; // skip "super"
            if after_keyword.is_empty() || after_keyword[0] == b'(' {
                return Some("a keyword");
            }
            // (super 1, 2) — plausible
            return None;
        }

        // ForwardingSuperNode — bare `super` with no args
        if inner.as_forwarding_super_node().is_some() {
            return Some("a keyword");
        }

        // return — keyword
        if let Some(ret) = inner.as_return_node() {
            let args = ret.arguments().map(|a| a.arguments().len()).unwrap_or(0);
            if args == 0 {
                return Some("a keyword");
            }
            // (return(1)) — has parenthesized single arg → keyword
            // (return 1, 2) — plausible
            let loc = inner.location();
            let src = &self.source.content[loc.start_offset()..loc.end_offset()];
            let after_keyword = &src[6..]; // skip "return"
            if !after_keyword.is_empty() && after_keyword[0] == b'(' {
                return Some("a keyword");
            }
            return None;
        }

        None
    }

    /// Check one-line rescue: (foo rescue bar)
    /// Flagged in most contexts, but not in ternary, conditional condition,
    /// array, hash, or method argument.
    fn check_one_line_rescue(
        &self,
        node: &ruby_prism::ParenthesesNode<'_>,
        parent: Option<&ParentInfo>,
    ) -> Option<&'static str> {
        // Not flagged in ternary
        if self.has_ternary_ancestor() {
            return None;
        }

        if self.is_in_conditional_predicate(node) {
            return None;
        }

        if let Some(p) = parent {
            match p.kind {
                // Not flagged in array or hash value
                ParentKind::Array | ParentKind::Pair => return None,
                // Not flagged in method call (method arg)
                ParentKind::Call => return None,
                // Not flagged when inside another rescue modifier (rescue in rescue)
                // RuboCop's `rescue?(node)` checks `^resbody` ancestor
                ParentKind::RescueModifier => return None,
                _ => {}
            }
        }

        Some("a one-line rescue")
    }

    /// Check if this parenthesized node is an argument of a parenthesized method call.
    /// RuboCop's argument_of_parenthesized_method_call? flags things like x.y((z)).
    fn check_argument_of_parenthesized_call(
        &self,
        node: &ruby_prism::ParenthesesNode<'_>,
        inner: &ruby_prism::Node<'_>,
        parent: Option<&ParentInfo>,
    ) -> Option<&'static str> {
        let p = parent?;
        if !matches!(p.kind, ParentKind::Call) {
            return None;
        }
        if !p.call_parenthesized {
            return None;
        }

        // If the paren is chained (followed by `.` or `&.`), it's the receiver of
        // the parent call, not an argument. RuboCop checks `parent.receiver != begin_node`.
        if is_chained(&self.source.content, node) {
            return None;
        }

        // Don't flag if inner is a basic conditional (if/unless/while/until modifier)
        if inner.as_if_node().is_some()
            || inner.as_unless_node().is_some()
            || inner.as_while_node().is_some()
            || inner.as_until_node().is_some()
        {
            return None;
        }

        // Don't flag rescue in method arg
        if inner.as_rescue_modifier_node().is_some() {
            return None;
        }

        // Don't flag pattern matching in method arg (RuboCop's in_pattern_matching_in_method_argument?)
        if inner.as_match_predicate_node().is_some() || inner.as_match_required_node().is_some() {
            return None;
        }

        // Don't flag if inner is a method call with unparenthesized args
        // where removing parens would change parsing.
        // But DO flag operator expressions like (z + w) since they don't need parens.
        if let Some(call) = inner.as_call_node() {
            if method_call_parentheses_required_in_method_arg(&call) {
                return None;
            }
        }

        Some("a method argument")
    }

    /// Check if inner is a one-line pattern matching expression (MatchPredicateNode or
    /// MatchRequiredNode). RuboCop flags these at top level / in method bodies, but exempts
    /// them in method args, boolean operators, assignments, and endless defs.
    fn check_pattern_matching(
        &self,
        inner: &ruby_prism::Node<'_>,
        parent: Option<&ParentInfo>,
    ) -> Option<&'static str> {
        if inner.as_match_predicate_node().is_none() && inner.as_match_required_node().is_none() {
            return None;
        }

        // Not flagged in method argument
        if parent.is_some_and(|p| matches!(p.kind, ParentKind::Call)) {
            return None;
        }

        // Not flagged if any ancestor is an operator keyword (&&, ||, and, or)
        for i in (0..self.parent_stack.len().saturating_sub(1)).rev() {
            if matches!(self.parent_stack[i].kind, ParentKind::And | ParentKind::Or) {
                return None;
            }
        }

        // Not flagged in endless def — check if a Def ancestor with `is_endless` flag
        for i in (0..self.parent_stack.len().saturating_sub(1)).rev() {
            if matches!(self.parent_stack[i].kind, ParentKind::Def)
                && self.parent_stack[i].is_endless_def
            {
                return None;
            }
        }

        // Not flagged in assignment context — assignments map to ParentKind::Other
        // but we track them with `is_assignment_parent`
        if parent.is_some_and(|p| p.is_assignment_parent) {
            return None;
        }

        Some("a one-line pattern matching")
    }

    /// Check if the parent is an interpolation (EmbeddedStatementsNode inside a dstr).
    fn is_interpolation(&self, parent: Option<&ParentInfo>) -> bool {
        parent.is_some_and(|p| matches!(p.kind, ParentKind::Interpolation))
    }

    fn push_parent(&mut self, kind: ParentKind, node_start_offset: usize) {
        self.parent_stack.push(ParentInfo {
            kind,
            multiline: false,
            single_child: false,
            is_statements_node: false,
            is_parentheses_body: false,
            is_parentheses_node: false,
            call_parenthesized: false,
            call_arg_count: 0,
            is_operator: false,
            is_unary_plus_minus: false,
            is_match_operator: false,
            is_endless_def: false,
            is_assignment_parent: false,
            node_start_offset,
            call_receiver_start_offset: None,
            call_first_arg_start_offset: None,
            call_first_arg_is_begin: false,
            statements_child_count: 0,
            rescue_expression_start_offset: None,
            conditional_predicate_range: None,
        });
    }

    /// RuboCop's `begin_node.chained?`: true when the ParenthesesNode is the
    /// receiver of its parent Call (including operators and unary calls).
    fn is_receiver_of_parent_call(
        &self,
        node: &ruby_prism::ParenthesesNode<'_>,
        parent: Option<&ParentInfo>,
    ) -> bool {
        if let Some(p) = parent {
            if matches!(p.kind, ParentKind::Call) {
                if let Some(recv_start) = p.call_receiver_start_offset {
                    return node.location().start_offset() == recv_start;
                }
            }
        }
        false
    }

    fn is_in_conditional_predicate(&self, node: &ruby_prism::ParenthesesNode<'_>) -> bool {
        let start = node.location().start_offset();
        let end = node.location().end_offset();

        for i in (0..self.parent_stack.len().saturating_sub(1)).rev() {
            let info = &self.parent_stack[i];
            match info.kind {
                ParentKind::Other => continue,
                ParentKind::If | ParentKind::While | ParentKind::Until | ParentKind::Case => {
                    // RuboCop checks `parent.condition == begin_node` — exact identity,
                    // not containment. A rescue nested INSIDE the predicate (e.g.,
                    // `if (var = (expr rescue nil))`) should still be flagged.
                    return info.conditional_predicate_range.is_some_and(
                        |(pred_start, pred_end)| start == pred_start && end == pred_end,
                    );
                }
                _ => return false,
            }
        }

        false
    }
}

fn check_logical<'a>(
    content: &[u8],
    paren_node: &ruby_prism::ParenthesesNode<'_>,
    inner: &ruby_prism::Node<'_>,
    parent: Option<&ParentInfo>,
    is_receiver: bool,
) -> Option<&'a str> {
    if is_receiver || is_chained(content, paren_node) {
        return None;
    }

    let is_and = inner.as_and_node().is_some();

    // RuboCop: semantic_operator? means keyword form (and/or);
    // if keyword form and has parent, skip
    if uses_keyword_operator(inner) && parent.is_some() {
        return None;
    }

    // ALLOWED_NODE_TYPES: or, send (call), splat, kwsplat
    if let Some(p) = parent {
        if matches!(
            p.kind,
            ParentKind::Or | ParentKind::Call | ParentKind::Splat | ParentKind::KeywordSplat
        ) {
            return None;
        }
    }

    // inner is `or` and parent is `and` → skip
    if !is_and {
        if let Some(p) = parent {
            if matches!(p.kind, ParentKind::And) {
                return None;
            }
        }
    }

    // ternary parent → skip
    if let Some(p) = parent {
        if matches!(p.kind, ParentKind::Ternary) {
            return None;
        }
    }

    Some("a logical expression")
}

fn check_method_call<'a>(
    content: &[u8],
    paren_node: &ruby_prism::ParenthesesNode<'_>,
    inner: &ruby_prism::Node<'_>,
    parent: Option<&ParentInfo>,
    is_receiver: bool,
) -> Option<&'a str> {
    let call = inner.as_call_node()?;

    // Check for unary operations first: !x, ~x, -x, +x
    if is_unary_operation(&call) {
        return check_unary(content, paren_node, inner, parent, is_receiver);
    }

    // RuboCop's check_send does NOT check chained? for method calls.
    // The chained? check only applies to logical/comparison expressions and
    // unary operations. Method calls like (foo.bar).to_s are flagged even
    // when chained, because removing the parens doesn't change the parse.

    // prefix_not: !expr — don't flag as method call (handled by unary check above)
    if call.name().as_slice() == b"!" && call.receiver().is_some() && call.arguments().is_none() {
        return None;
    }

    // If the inner call has a do..end block (or a descendant with do..end block
    // in a method chain), parens may be required.
    if has_do_end_block_in_chain(&call) {
        return None;
    }

    // call_chain_starts_with_int? — if the call chain starts with an int
    // and the parent is a unary +/- operation, parens are needed.
    // e.g., -(1.foo) — removing parens gives -1.foo which parses as (-1).foo
    if call_chain_starts_with_int_from_call(&call)
        && is_receiver
        && parent.is_some_and(|p| p.is_unary_plus_minus)
    {
        return None;
    }

    let has_args = call.arguments().is_some();
    // Block arguments (&block) count as args for the singular-parent check.
    // `(method_args.map &:to_json).join(',')` — removing parens changes parsing.
    let has_block_arg = call
        .block()
        .is_some_and(|b| b.as_block_argument_node().is_some());
    let call_has_parens = call.opening_loc().is_some_and(|loc| loc.as_slice() == b"(");
    let is_square_brackets = call.name().as_slice() == b"[]" && call.call_operator_loc().is_none();

    // RuboCop does not fall back to "a method call" for comparisons used as the
    // direct return value of a method or block body, but it still does for nested
    // begin/paren contexts like `((1 == 2)).to_s`.
    if is_comparison(inner)
        && parent.is_some_and(|p| matches!(p.kind, ParentKind::Other) && !p.is_parentheses_body)
    {
        return None;
    }

    // RuboCop's method_call_with_redundant_parentheses?:
    // If the inner call has unparenthesized args (like operators `a + b`),
    // only flag in "singular parent" positions where the paren is the sole
    // content of its parent. Otherwise removing parens would change parsing.
    // RuboCop checks `begin_node.parent.children.one?` — the parent must have
    // exactly one child. For Return/Next/Break/Super/Yield, this means the
    // keyword has a single argument. `[]` calls are handled like RuboCop's
    // `square_brackets?` matcher and don't need the singular-parent check.
    if (has_args || has_block_arg)
        && !call_has_parens
        && !is_square_brackets
        && !has_singular_parenthesized_parent(parent)
    {
        return None;
    }

    Some("a method call")
}

/// Check if the first receiver in a call chain is an integer literal.
fn call_chain_starts_with_int_from_call(call: &ruby_prism::CallNode<'_>) -> bool {
    if let Some(recv) = call.receiver() {
        call_chain_starts_with_int(&recv)
    } else {
        false
    }
}

/// Check unary operation: (!x), (~x), (-x), (+x)
fn check_unary<'a>(
    content: &[u8],
    paren_node: &ruby_prism::ParenthesesNode<'_>,
    inner: &ruby_prism::Node<'_>,
    parent: Option<&ParentInfo>,
    is_receiver: bool,
) -> Option<&'a str> {
    // RuboCop: `return if begin_node.chained?`
    if is_receiver || is_chained(content, paren_node) {
        return None;
    }

    let call = inner.as_call_node()?;
    let name = call.name().as_slice();

    // RuboCop's method_call_with_redundant_parentheses? returns false for
    // prefix_not? nodes. This means ALL (!expr) patterns are exempt from
    // the unary check, because `suspect_unary?` doesn't unwrap through `!`
    // and then `prefix_not?` causes an early return.
    if name == b"!" {
        return None;
    }

    // For unary -/+ on method chain starting with int: -(1.foo) is plausible
    if matches!(name, b"-@" | b"+@") {
        if let Some(recv) = call.receiver() {
            if call_chain_starts_with_int(&recv) {
                return None;
            }
        }
    }

    // RuboCop's check_unary unwraps nested unary ops (except prefix_not),
    // then calls method_call_with_redundant_parentheses? on the result.
    // Only flag if the unwrapped base is actually a method call
    // (send/super/yield/defined?). If the base is a variable, literal,
    // or parens node, don't flag.
    if let Some(recv) = call.receiver() {
        // Unwrap nested unary operations to find the base receiver
        // (RuboCop: `node = node.children.first while suspect_unary?(node)`)
        let mut current = recv;
        while let Some(inner_call) = current.as_call_node() {
            // suspect_unary? is send_type? && unary_operation? && !prefix_not?
            if is_unary_operation(&inner_call) && inner_call.name().as_slice() != b"!" {
                if let Some(r) = inner_call.receiver() {
                    current = r;
                    continue;
                }
            }
            break;
        }
        // If the base is a ParenthesesNode (begin node), the outer
        // parens are needed (e.g., (!(x = expr))).
        if current.as_parentheses_node().is_some() {
            return None;
        }
        // RuboCop's method_call_with_redundant_parentheses? requires the node
        // to be a call/super/yield/defined?. If the base is a variable, literal,
        // constant, or anything else, don't flag. E.g., (-num) % 4, +(-v).
        if current.as_call_node().is_none()
            && current.as_super_node().is_none()
            && current.as_forwarding_super_node().is_none()
            && current.as_yield_node().is_none()
            && current.as_defined_node().is_none()
        {
            return None;
        }
    }

    Some("a unary operation")
}

fn is_unary_operation(call: &ruby_prism::CallNode<'_>) -> bool {
    let name = call.name().as_slice();
    // Unary: !, ~, -@ (unary minus), +@ (unary plus)
    if !matches!(name, b"!" | b"~" | b"-@" | b"+@") {
        return false;
    }
    // Must have a receiver and no arguments (unary prefix)
    call.receiver().is_some() && call.arguments().is_none() && call.opening_loc().is_none()
}

/// Check if a method call chain starts with an integer literal.
/// E.g., `1.foo` or `1.foo.bar`
fn call_chain_starts_with_int(node: &ruby_prism::Node<'_>) -> bool {
    if node.as_integer_node().is_some() {
        return true;
    }
    if let Some(call) = node.as_call_node() {
        if let Some(recv) = call.receiver() {
            return call_chain_starts_with_int(&recv);
        }
    }
    false
}

fn is_chained(content: &[u8], paren_node: &ruby_prism::ParenthesesNode<'_>) -> bool {
    let end_offset = paren_node.location().end_offset();
    // Skip whitespace (including newlines) after the closing paren to find `.` or `&.`.
    // This handles multiline chains like:
    //   (expr)
    //     .method
    let mut i = end_offset;
    while i < content.len() && matches!(content[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    if i < content.len() {
        // `.method` (dot chaining)
        if content[i] == b'.' {
            return true;
        }
        // `&.method` (safe navigation) — must be `&.` not `&&` or `&` alone
        if content[i] == b'&' && i + 1 < content.len() && content[i + 1] == b'.' {
            return true;
        }
    }
    false
}

/// Returns true if the call node has a do..end block attached to it.
fn has_do_end_block(call: &ruby_prism::CallNode<'_>) -> bool {
    if let Some(block) = call.block() {
        if let Some(block_node) = block.as_block_node() {
            return block_node.opening_loc().as_slice() == b"do";
        }
    }
    false
}

/// Check if any call in the chain has a do..end block.
/// This handles cases like `(baz do ... end.qux)` in keyword arguments.
fn has_do_end_block_in_chain(call: &ruby_prism::CallNode<'_>) -> bool {
    if has_do_end_block(call) {
        return true;
    }
    // Check receiver chain
    if let Some(recv) = call.receiver() {
        if let Some(recv_call) = recv.as_call_node() {
            return has_do_end_block_in_chain(&recv_call);
        }
    }
    false
}

fn uses_keyword_operator(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(and_node) = node.as_and_node() {
        predicate_operator_predicates::is_semantic_and(&and_node)
    } else if let Some(or_node) = node.as_or_node() {
        predicate_operator_predicates::is_semantic_or(&or_node)
    } else {
        false
    }
}

fn is_operator_method(call: &ruby_prism::CallNode<'_>) -> bool {
    method_identifier_predicates::is_operator_method(call.name().as_slice())
}

fn method_call_parentheses_required_in_method_arg(call: &ruby_prism::CallNode<'_>) -> bool {
    if call.block().is_some() {
        return false;
    }

    let has_args = call.arguments().is_some();
    let call_has_parens = call.opening_loc().is_some_and(|loc| loc.as_slice() == b"(");
    has_args
        && !call_has_parens
        && (call.receiver().is_none() || call.call_operator_loc().is_some())
}

fn has_singular_parenthesized_parent(parent: Option<&ParentInfo>) -> bool {
    match parent {
        None => true,
        Some(parent) => {
            if matches!(parent.kind, ParentKind::Splat | ParentKind::KeywordSplat) {
                return false;
            }

            if matches!(
                parent.kind,
                ParentKind::Return
                    | ParentKind::Next
                    | ParentKind::Break
                    | ParentKind::Super
                    | ParentKind::Yield
            ) {
                return parent.call_arg_count == 1;
            }

            parent.single_child
        }
    }
}

fn classify_simple(node: &ruby_prism::Node<'_>) -> Option<&'static str> {
    if is_literal(node) {
        Some("a literal")
    } else if is_variable(node) {
        Some("a variable")
    } else if is_keyword_value(node) {
        Some("a keyword")
    } else if is_constant(node) {
        Some("a constant")
    } else {
        None
    }
}

fn is_literal(node: &ruby_prism::Node<'_>) -> bool {
    node.as_string_node().is_some()
        || node.as_interpolated_string_node().is_some()
        || node.as_x_string_node().is_some()
        || node.as_interpolated_x_string_node().is_some()
        || node.as_symbol_node().is_some()
        || node.as_interpolated_symbol_node().is_some()
        || node.as_integer_node().is_some()
        || node.as_float_node().is_some()
        || node.as_rational_node().is_some()
        || node.as_imaginary_node().is_some()
        || node.as_hash_node().is_some()
        || node.as_keyword_hash_node().is_some()
        || node.as_array_node().is_some()
        || node.as_nil_node().is_some()
        || node.as_true_node().is_some()
        || node.as_false_node().is_some()
        || node.as_regular_expression_node().is_some()
        || node.as_interpolated_regular_expression_node().is_some()
}

fn is_xstring(node: &ruby_prism::Node<'_>) -> bool {
    node.as_x_string_node().is_some() || node.as_interpolated_x_string_node().is_some()
}

fn is_variable(node: &ruby_prism::Node<'_>) -> bool {
    node.as_local_variable_read_node().is_some()
        || node.as_instance_variable_read_node().is_some()
        || node.as_class_variable_read_node().is_some()
        || node.as_global_variable_read_node().is_some()
}

fn is_keyword_value(node: &ruby_prism::Node<'_>) -> bool {
    node.as_self_node().is_some()
        || node.as_source_file_node().is_some()
        || node.as_source_line_node().is_some()
        || node.as_source_encoding_node().is_some()
}

fn is_assignment(node: &ruby_prism::Node<'_>) -> bool {
    // Variable write nodes
    if node.as_local_variable_write_node().is_some()
        || node.as_instance_variable_write_node().is_some()
        || node.as_class_variable_write_node().is_some()
        || node.as_global_variable_write_node().is_some()
        || node.as_constant_write_node().is_some()
        || node.as_constant_path_write_node().is_some()
    {
        return true;
    }
    // Compound assignment operators (||=, &&=, +=, etc.) on variables
    if node.as_local_variable_or_write_node().is_some()
        || node.as_local_variable_and_write_node().is_some()
        || node.as_local_variable_operator_write_node().is_some()
        || node.as_instance_variable_or_write_node().is_some()
        || node.as_instance_variable_and_write_node().is_some()
        || node.as_instance_variable_operator_write_node().is_some()
        || node.as_class_variable_or_write_node().is_some()
        || node.as_class_variable_and_write_node().is_some()
        || node.as_class_variable_operator_write_node().is_some()
        || node.as_global_variable_or_write_node().is_some()
        || node.as_global_variable_and_write_node().is_some()
        || node.as_global_variable_operator_write_node().is_some()
        || node.as_constant_or_write_node().is_some()
        || node.as_constant_and_write_node().is_some()
        || node.as_constant_operator_write_node().is_some()
        || node.as_constant_path_or_write_node().is_some()
        || node.as_constant_path_and_write_node().is_some()
        || node.as_constant_path_operator_write_node().is_some()
    {
        return true;
    }
    // Index compound assignment: a[b] ||=, a[b] &&=, a[b] +=
    if node.as_index_or_write_node().is_some()
        || node.as_index_and_write_node().is_some()
        || node.as_index_operator_write_node().is_some()
    {
        return true;
    }
    false
}

fn is_assignment_call(node: &ruby_prism::Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        let name = call.name().as_slice();
        name.ends_with(b"=") && !matches!(name, b"==" | b"!=" | b"<=" | b">=" | b"===")
    })
}

fn is_comparison(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        let name = call.name().as_slice();
        matches!(
            name,
            b"==" | b"!=" | b"<" | b">" | b"<=" | b">=" | b"<=>" | b"==="
        )
    } else {
        false
    }
}

fn is_constant(node: &ruby_prism::Node<'_>) -> bool {
    node.as_constant_read_node().is_some() || node.as_constant_path_node().is_some()
}

/// Check if inner is a lambda_or_proc with braces (not do..end).
/// (-> { x }), (lambda { x }), (proc { x })
fn is_lambda_or_proc_with_braces(node: &ruby_prism::Node<'_>) -> bool {
    // Lambda literal: -> { x } is a LambdaNode in Prism
    if let Some(lambda) = node.as_lambda_node() {
        // Check if it uses { } (not do..end)
        return lambda.opening_loc().as_slice() == b"{";
    }

    // lambda { x } and proc { x } are CallNode with a block
    if let Some(call) = node.as_call_node() {
        let name = call.name().as_slice();
        if (name == b"lambda" || name == b"proc")
            && call.receiver().is_none()
            && call.arguments().is_none()
        {
            if let Some(block) = call.block() {
                if let Some(block_node) = block.as_block_node() {
                    return block_node.opening_loc().as_slice() == b"{";
                }
            }
        }
    }

    false
}

/// Check if a negative numeric literal is raised to a power.
/// (-2)**2 needs parens, so we should NOT flag the literal.
fn is_raised_to_power_negative_numeric(
    inner: &ruby_prism::Node<'_>,
    paren_node: &ruby_prism::ParenthesesNode<'_>,
    content: &[u8],
) -> bool {
    // Check if inner is a negative numeric (IntegerNode or FloatNode).
    // Prism parses `-2` directly as IntegerNode with negative value.
    // We check the source text to see if it starts with `-`.
    let is_negative_numeric =
        if inner.as_integer_node().is_some() || inner.as_float_node().is_some() {
            let loc = inner.location();
            loc.start_offset() < content.len() && content[loc.start_offset()] == b'-'
        } else if let Some(call) = inner.as_call_node() {
            // Also handle Prism representing `-2` as CallNode with name `-@`
            call.name().as_slice() == b"-@"
                && call
                    .receiver()
                    .is_some_and(|r| r.as_integer_node().is_some() || r.as_float_node().is_some())
        } else {
            false
        };

    if !is_negative_numeric {
        return false;
    }

    // Check if the closing paren is followed by ** (possibly with whitespace)
    let end_offset = paren_node.location().end_offset();
    let mut i = end_offset;
    while i < content.len() && content[i] == b' ' {
        i += 1;
    }
    if i + 1 < content.len() {
        return content[i] == b'*' && content[i + 1] == b'*';
    }
    false
}

impl<'pr> Visit<'pr> for RedundantParensVisitor<'_> {
    // visit_branch_node_enter/leave provide push/pop for ALL branch nodes.
    // Specific visit_* methods then MODIFY the top of stack to set the correct kind.
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.push_parent(ParentKind::Other, node.location().start_offset());
    }

    fn visit_branch_node_leave(&mut self) {
        self.parent_stack.pop();
    }

    fn visit_statements_node(&mut self, node: &ruby_prism::StatementsNode<'pr>) {
        let is_parentheses_body = self.parent_stack.len() >= 2
            && self.parent_stack[self.parent_stack.len() - 2].is_parentheses_node;
        if let Some(top) = self.parent_stack.last_mut() {
            top.is_statements_node = true;
            top.single_child = node.body().len() == 1 && is_parentheses_body;
            top.is_parentheses_body = is_parentheses_body;
            top.statements_child_count = node.body().len();
        }
        ruby_prism::visit_statements_node(self, node);
    }

    fn visit_parentheses_node(&mut self, node: &ruby_prism::ParenthesesNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.is_parentheses_node = true;
        }
        self.check_parens(node);
        // enter already pushed; leave will pop
        ruby_prism::visit_parentheses_node(self, node);
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            let start_line = self
                .source
                .offset_to_line_col(node.location().start_offset())
                .0;
            let end_line = self
                .source
                .offset_to_line_col(node.location().end_offset().saturating_sub(1))
                .0;
            top.kind = ParentKind::Call;
            top.multiline = start_line != end_line;
            top.call_parenthesized = node.opening_loc().is_some_and(|loc| loc.as_slice() == b"(");
            top.call_arg_count = node.arguments().map(|a| a.arguments().len()).unwrap_or(0);
            top.is_operator = is_operator_method(node);
            top.is_unary_plus_minus =
                matches!(node.name().as_slice(), b"-@" | b"+@") && node.arguments().is_none();
            top.is_match_operator = node.name().as_slice() == b"=~";
            top.is_assignment_parent = node.equal_loc().is_some();
            top.call_receiver_start_offset = node.receiver().map(|r| r.location().start_offset());
            top.call_first_arg_start_offset = node
                .arguments()
                .and_then(|args| args.arguments().iter().next())
                .map(|first| first.location().start_offset());
            // RuboCop's like_method_argument_parentheses? checks node.first_argument.begin_type?
            top.call_first_arg_is_begin = node
                .arguments()
                .and_then(|args| args.arguments().iter().next())
                .is_some_and(|first| first.as_parentheses_node().is_some());
        }
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::And;
        }
        ruby_prism::visit_and_node(self, node);
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Or;
        }
        ruby_prism::visit_or_node(self, node);
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            if node.if_keyword_loc().is_none() {
                top.kind = ParentKind::Ternary;
            } else {
                top.kind = ParentKind::If;
                top.conditional_predicate_range = Some((
                    node.predicate().location().start_offset(),
                    node.predicate().location().end_offset(),
                ));
            }
        }
        ruby_prism::visit_if_node(self, node);
    }

    fn visit_else_node(&mut self, node: &ruby_prism::ElseNode<'pr>) {
        // In Prism, ElseNode sits between IfNode and its false branch.
        // In Parser AST (RuboCop), there's no ElseNode — begin_node.parent
        // goes directly to the IfNode. Propagate Ternary kind so that
        // the false branch sees Ternary as its parent, matching RuboCop behavior.
        if self.parent_stack.len() >= 2 {
            let parent_kind = self.parent_stack[self.parent_stack.len() - 2].kind;
            if matches!(parent_kind, ParentKind::Ternary) {
                if let Some(top) = self.parent_stack.last_mut() {
                    top.kind = ParentKind::Ternary;
                }
            }
        }
        ruby_prism::visit_else_node(self, node);
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::If; // treat unless same as if for conditional ancestor check
            top.conditional_predicate_range = Some((
                node.predicate().location().start_offset(),
                node.predicate().location().end_offset(),
            ));
        }
        ruby_prism::visit_unless_node(self, node);
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::While;
            top.single_child = node.statements().is_none();
            top.conditional_predicate_range = Some((
                node.predicate().location().start_offset(),
                node.predicate().location().end_offset(),
            ));
        }
        ruby_prism::visit_while_node(self, node);
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Until;
            top.conditional_predicate_range = Some((
                node.predicate().location().start_offset(),
                node.predicate().location().end_offset(),
            ));
        }
        ruby_prism::visit_until_node(self, node);
    }

    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Case;
            top.conditional_predicate_range = node.predicate().map(|predicate| {
                (
                    predicate.location().start_offset(),
                    predicate.location().end_offset(),
                )
            });
        }
        ruby_prism::visit_case_node(self, node);
    }

    fn visit_return_node(&mut self, node: &ruby_prism::ReturnNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            let start_line = self
                .source
                .offset_to_line_col(node.location().start_offset())
                .0;
            let end_line = self
                .source
                .offset_to_line_col(node.location().end_offset().saturating_sub(1))
                .0;
            top.kind = ParentKind::Return;
            top.multiline = start_line != end_line;
            top.call_arg_count = node.arguments().map(|a| a.arguments().len()).unwrap_or(0);
        }
        ruby_prism::visit_return_node(self, node);
    }

    fn visit_next_node(&mut self, node: &ruby_prism::NextNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            let start_line = self
                .source
                .offset_to_line_col(node.location().start_offset())
                .0;
            let end_line = self
                .source
                .offset_to_line_col(node.location().end_offset().saturating_sub(1))
                .0;
            top.kind = ParentKind::Next;
            top.multiline = start_line != end_line;
            top.call_arg_count = node.arguments().map(|a| a.arguments().len()).unwrap_or(0);
        }
        ruby_prism::visit_next_node(self, node);
    }

    fn visit_break_node(&mut self, node: &ruby_prism::BreakNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            let start_line = self
                .source
                .offset_to_line_col(node.location().start_offset())
                .0;
            let end_line = self
                .source
                .offset_to_line_col(node.location().end_offset().saturating_sub(1))
                .0;
            top.kind = ParentKind::Break;
            top.multiline = start_line != end_line;
            top.call_arg_count = node.arguments().map(|a| a.arguments().len()).unwrap_or(0);
        }
        ruby_prism::visit_break_node(self, node);
    }

    fn visit_splat_node(&mut self, node: &ruby_prism::SplatNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Splat;
        }
        ruby_prism::visit_splat_node(self, node);
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Block;
        }
        ruby_prism::visit_block_node(self, node);
    }

    fn visit_block_argument_node(&mut self, node: &ruby_prism::BlockArgumentNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::BlockArgument;
            // BlockArgumentNode wraps a single expression (the value after &)
            top.single_child = true;
        }
        ruby_prism::visit_block_argument_node(self, node);
    }

    fn visit_rescue_modifier_node(&mut self, node: &ruby_prism::RescueModifierNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::RescueModifier;
            top.rescue_expression_start_offset =
                Some(node.rescue_expression().location().start_offset());
        }
        ruby_prism::visit_rescue_modifier_node(self, node);
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::MultipleAssignment;
        }
        ruby_prism::visit_multi_write_node(self, node);
    }

    fn visit_assoc_splat_node(&mut self, node: &ruby_prism::AssocSplatNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::KeywordSplat;
        }
        ruby_prism::visit_assoc_splat_node(self, node);
    }

    fn visit_range_node(&mut self, node: &ruby_prism::RangeNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Range;
        }
        ruby_prism::visit_range_node(self, node);
    }

    fn visit_yield_node(&mut self, node: &ruby_prism::YieldNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            let start_line = self
                .source
                .offset_to_line_col(node.location().start_offset())
                .0;
            let end_line = self
                .source
                .offset_to_line_col(node.location().end_offset().saturating_sub(1))
                .0;
            top.kind = ParentKind::Yield;
            top.multiline = start_line != end_line;
            top.call_parenthesized = node.lparen_loc().is_some();
            top.call_arg_count = node.arguments().map(|a| a.arguments().len()).unwrap_or(0);
        }
        ruby_prism::visit_yield_node(self, node);
    }

    fn visit_super_node(&mut self, node: &ruby_prism::SuperNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            let start_line = self
                .source
                .offset_to_line_col(node.location().start_offset())
                .0;
            let end_line = self
                .source
                .offset_to_line_col(node.location().end_offset().saturating_sub(1))
                .0;
            top.kind = ParentKind::Super;
            top.multiline = start_line != end_line;
            top.call_parenthesized = node.lparen_loc().is_some();
            top.call_arg_count = node.arguments().map(|a| a.arguments().len()).unwrap_or(0);
        }
        ruby_prism::visit_super_node(self, node);
    }

    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Array;
            top.single_child = node.elements().len() == 1;
        }
        ruby_prism::visit_array_node(self, node);
    }

    fn visit_hash_node(&mut self, node: &ruby_prism::HashNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Hash;
        }
        ruby_prism::visit_hash_node(self, node);
    }

    fn visit_keyword_hash_node(&mut self, node: &ruby_prism::KeywordHashNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Hash;
        }
        ruby_prism::visit_keyword_hash_node(self, node);
    }

    fn visit_assoc_node(&mut self, node: &ruby_prism::AssocNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Pair;
        }
        ruby_prism::visit_assoc_node(self, node);
    }

    fn visit_optional_parameter_node(&mut self, node: &ruby_prism::OptionalParameterNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Parameter;
        }
        ruby_prism::visit_optional_parameter_node(self, node);
    }

    fn visit_optional_keyword_parameter_node(
        &mut self,
        node: &ruby_prism::OptionalKeywordParameterNode<'pr>,
    ) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Parameter;
        }
        ruby_prism::visit_optional_keyword_parameter_node(self, node);
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::SingletonClass;
        }
        ruby_prism::visit_singleton_class_node(self, node);
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Def;
            // Endless defs have an `equal_loc` (the `=` sign)
            top.is_endless_def = node.equal_loc().is_some();
        }
        ruby_prism::visit_def_node(self, node);
    }

    fn visit_embedded_statements_node(&mut self, node: &ruby_prism::EmbeddedStatementsNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.kind = ParentKind::Interpolation;
        }
        ruby_prism::visit_embedded_statements_node(self, node);
    }

    // Assignment nodes: track for pattern matching exemption
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.is_assignment_parent = true;
        }
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.is_assignment_parent = true;
        }
        ruby_prism::visit_local_variable_or_write_node(self, node);
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.is_assignment_parent = true;
        }
        ruby_prism::visit_local_variable_and_write_node(self, node);
    }

    fn visit_instance_variable_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableWriteNode<'pr>,
    ) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.is_assignment_parent = true;
        }
        ruby_prism::visit_instance_variable_write_node(self, node);
    }

    fn visit_class_variable_write_node(&mut self, node: &ruby_prism::ClassVariableWriteNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.is_assignment_parent = true;
        }
        ruby_prism::visit_class_variable_write_node(self, node);
    }

    fn visit_global_variable_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableWriteNode<'pr>,
    ) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.is_assignment_parent = true;
        }
        ruby_prism::visit_global_variable_write_node(self, node);
    }

    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.is_assignment_parent = true;
        }
        ruby_prism::visit_constant_write_node(self, node);
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        if let Some(top) = self.parent_stack.last_mut() {
            top.is_assignment_parent = true;
        }
        ruby_prism::visit_local_variable_operator_write_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(RedundantParentheses, "cops/style/redundant_parentheses");
}
