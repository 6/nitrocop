use crate::cop::shared::method_dispatch_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Matches RuboCop's guarded receiver handling for modifier `if`/`unless`, adjacent
/// clauses inside chained `&&` guards like `proof && dom_body && dom_body.include?(proof)`,
/// and ternaries such as `uri.port = port ? port.to_i : nil`.
/// The current fixes also match RuboCop more closely by:
/// recognizing checked receivers that already use safe navigation (`foo&.bar && foo.bar.baz`);
/// searching parenthesized `||` / nested `&&` RHS trees for the first safe receiver
/// invocation (`handler && (!handler.is_a?(Proc) || (handler.lambda? && ...))`);
/// limiting direct-argument suppression to the expression itself so nested block
/// bodies, hash values, and `if`/`unless` expressions inside arguments still report;
/// and preserving outer ruby2js-style `async` send ancestors for ordinary block bodies
/// while still resetting nested block literals reached through call arguments.
/// `send`/`public_send` arguments remain skipped for plain-variable guards, but call
/// receivers like `hash["kind"] && hash["kind"].downcase.to_sym` now match RuboCop.
/// Direct receiver cases like `(foo && foo.bar).to_s` remain skipped, as do unsafe
/// plain-variable dynamic-send arguments, double-colon direct arguments, negated
/// wrapper contexts, and `async { ... }[]` / `async def ... end` wrappers where RuboCop
/// keeps the outer unsafe send ancestry.
///
/// Recent fixes (FP/FN alignment):
/// - Ternaries inside call arguments are no longer blanket-suppressed by dotless/unsafe
///   parent calls. RuboCop's `unsafe_method?` returns false for ternary nodes (except
///   negation), so ternaries inside `format_elements(...)`, `ssl_request(...)`, etc. are
///   now correctly flagged. Suppression remains for nil-method call arguments (e.g.,
///   `instance_variable_set`) where the ancestor walk finds a nil-responding method.
/// - Lambda bodies (`-> { ... }`) now reset `in_call_arguments` and increment `in_block`,
///   matching RuboCop's AST where lambdas are `:block` type nodes. This fixes FNs for
///   modifier-if patterns inside lambdas (e.g., `after_commit -> { foo.bar if foo }`).
/// - `visit_assoc_node` no longer resets `in_unsafe_parent` for hash values, fixing FPs
///   where `&&` inside lambda hash values of dotless calls (e.g.,
///   `before_save :foo, if: -> { x && x.bar }`) was incorrectly flagged.
/// - `in_unsafe_parent` suppression for `&&` patterns is now chain-length-aware.
///   RuboCop's `unsafe_method_used?` ancestor walk is bounded by the rhs expression
///   for chain length ≥ 2 (`break false if ancestor == method_chain`) but escapes for
///   chain length 1. We now only suppress `&&` when `in_unsafe_parent > 0` AND
///   `chain.len() == 1`, fixing FNs where chain ≥ 2 inside dotless calls was wrongly
///   suppressed (e.g., `record && record.orm_model.schema` inside method arguments).
/// - `visit_block_node` no longer resets `in_unsafe_parent` when entering blocks inside
///   call arguments. RuboCop's ancestor walk crosses block boundaries, so a dotless
///   parent like `puts(items.map { |x| x && x.b })` correctly suppresses chain-1
///   `&&` inside the block. Only `in_call_arguments`, `in_ternary_operator_parent`,
///   `in_assignment_or_operator_parent`, and `dotted_assignment_parent_starts` are reset.
/// - `nodes_match` no longer unwraps `ParenthesesNode` on the right (checked) side.
///   RuboCop's `find_matching_receiver_invocation` never unwraps parentheses, so when
///   comparing `CallNode` (body receiver) with `ParenthesesNode` (condition), the match
///   fails. Previously, nitrocop would unwrap the right side and recursively compare,
///   causing false positives like `obj.bar if (obj)` where RuboCop correctly skips
///   the parenthesized condition.
///
/// - Added `in_dotless_call_arguments` tracking to distinguish modifier-if/unless
///   suppression inside dotless vs dot call arguments. RuboCop suppresses modifier-if
///   only inside dotless call arguments (e.g., `scope`, `install_win`), not inside dot
///   call arguments (e.g., `foo.bar(if x then x.y end)`). Previously, nitrocop used
///   `in_call_arguments > 0` which blanket-suppressed all modifier-if in any call
///   arguments, causing false negatives for modifier-if inside dot call arguments
///   (e.g., `Match.start_match(..., if opponent_names then opponent_names.split(...) end)`).
///   The fix tracks dotless calls separately via `call_operator_loc().is_none()` and
///   uses `in_dotless_call_arguments` in modifier-if/unless skip checks. Also saved/restored
///   `in_dotless_call_arguments` in `visit_block_node` and `visit_lambda_node` to prevent
///   the flag from leaking into nested block bodies.
/// - `&&` suppression from nil-responding ancestors is now chain-length-aware. RuboCop's
///   ancestor walk can escape to outer `==` / nil-method ancestors only for chain length 1;
///   chain length >= 2 stops at the outermost call in the safe-navigation chain. This fixes
///   FNs such as `((product && product.population_counting.to_sym) == expected ? 1 : 0)`.
/// - Unary negation wrappers now search nested `&&` descendants inside the `!` receiver,
///   matching RuboCop's flattened `and` traversal for patterns like
///   `lat_lng && !(lat_lng.first.zero? && lat_lng.last.zero?)`.
/// - Nested `&&` searches now only consider the FIRST clause of an inner chain, matching
///   RuboCop's source-ordered `collect_and_clauses` pairing. This avoids false positives
///   when the guarded receiver appears only after another condition, such as
///   `foo && (cond && foo.bar?)` or `store_response && ((action == :tokens && store_response.match(...)) || ...)`.
/// - Modifier `if`/`unless` is no longer blanket-suppressed under assignment/operator
///   parents before the method chain is known. RuboCop only suppresses those contexts when
///   the chain length is 1 and the ancestor walk escapes to the outer unsafe parent, so
///   chain length >= 2 cases like `parts[:scheme] = (scheme.dup.force_encoding(...) if scheme)`
///   now report.
/// - `&&` inside `private def` / `private_class_method def` wrappers is no longer skipped
///   just because the definition itself is a call argument. The existing `in_unsafe_parent`
///   chain-length check already models RuboCop's real suppression boundary.
/// - Modifier `if`/`unless` with a block-call body now tracks only OUTER unsafe send ancestry
///   above the surrounding block. RuboCop's `unsafe_method_used?` walk can escape through the
///   enclosing block into outer unsafe container sends like `([Builder.new do ... end] + x)` or
///   `puts(items.map do ... end)`, but safe outer wrappers alone do not suppress. This keeps
///   the RDF CLI array case skipped while still flagging ordinary nested safe-block cases such as
///   `outer.wrap { inner.wrap { items.each { ... } if items } }`.
/// - Chain-length-1 matches found under a local `!` wrapper are treated as unsafe, matching
///   RuboCop's `unsafe_method_used?` walk for patterns like
///   `foo && !(foo.respond_to?(:empty?) && foo.empty?)` while preserving longer-chain
///   offenses such as `lat_lng && !(lat_lng.first.zero? && lat_lng.last.zero?)`.
pub struct SafeNavigation;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AndOperatorKind {
    KeywordAnd,
    DoubleAmpersand,
}

enum SafeChainSearchResult<'a> {
    NoneFound,
    UnsafeMatch,
    Safe(Vec<ruby_prism::CallNode<'a>>),
}

struct TernaryCheckContext<'a> {
    max_chain_length: usize,
    allowed_methods: &'a Option<Vec<String>>,
    offense_start_offset: Option<usize>,
    skip_nested_block_call_args: bool,
    skip_direct_receiver_block_body_block_calls: bool,
}

struct ModifierIfCheckContext<'a> {
    max_chain_length: usize,
    allowed_methods: &'a Option<Vec<String>>,
    skip_direct_receiver_block_body_block_calls: bool,
    /// Whether the modifier-if is inside an unsafe parent (dotless call, operator, etc.).
    /// RuboCop's `unsafe_method_used?` ancestor walk escapes the method chain for
    /// chain-length-1 and finds these parents, suppressing the offense.
    in_unsafe_parent: bool,
    /// Whether an outer assignment/operator ancestor remains above the modifier-if.
    /// Like RuboCop's ancestor walk, this only suppresses chain-length-1 cases.
    in_assignment_or_operator_parent: bool,
    /// Whether an unsafe send ancestor remains above the surrounding block node.
    /// For block-call bodies, RuboCop can escape through the enclosing block and
    /// reach outer unsafe container sends like `+` / `puts`, but safe outer calls
    /// alone do not suppress.
    has_outer_unsafe_parent_above_current_block: bool,
}

/// Methods that `nil` responds to in vanilla Ruby.
/// Converting `foo && foo.bar.is_a?(X)` to `foo&.bar&.is_a?(X)` changes behavior
/// because nil already responds to these methods.
///
/// For `&&` patterns, these suppress offenses when found in the method chain after
/// the checked receiver.  For ternary patterns, these suppress offenses when found
/// as ancestor calls (e.g. `(x.nil? ? nil : x.to_date) == actual` — `==` is a nil
/// method, so converting would change behavior since nil already responds to `==`).
const NIL_METHODS: &[&[u8]] = &[
    b"nil?",
    b"is_a?",
    b"kind_of?",
    b"instance_of?",
    b"respond_to?",
    b"eql?",
    b"equal?",
    b"frozen?",
    b"class",
    b"clone",
    b"dup",
    b"freeze",
    b"hash",
    b"inspect",
    b"to_s",
    b"to_a",
    b"to_f",
    b"to_i",
    b"to_r",
    b"to_c",
    b"to_json",
    b"object_id",
    b"send",
    b"__send__",
    b"__id__",
    b"public_send",
    b"tap",
    b"then",
    b"yield_self",
    b"itself",
    b"display",
    b"method",
    b"public_method",
    b"singleton_method",
    b"define_singleton_method",
    b"extend",
    b"pp",
    b"respond_to_missing?",
    b"instance_variable_get",
    b"instance_variable_set",
    b"instance_variable_defined?",
    b"instance_variables",
    b"remove_instance_variable",
    // Operators that nil responds to — needed so `in_nil_safe_call_ancestor` correctly
    // suppresses ternaries used as receivers of these operators (e.g. `(x ? x.y : nil) == z`).
    // RuboCop uses `nil.methods` at runtime which includes these.
    // NOTE: `!` is intentionally excluded.  It IS a nil method, but including it
    // here makes `is_nil_safe_call_ancestor` true for `!(...)` calls, which wrongly
    // suppresses `&&` patterns like `!(x && x.foo.bar)` that RuboCop flags.
    // `!` is already handled as an unsafe parent via `in_unsafe_parent` (chain-length-1).
    b"==",
    b"!=",
    b"===",
    b"<=>",
    b"&",
    b"|",
    b"^",
];

impl SafeNavigation {
    /// Check if a call node is a dotless operator method ([], []=, +, -, etc.)
    fn is_dotless_operator(call: &ruby_prism::CallNode<'_>) -> bool {
        // If there's a dot/call operator, it's not a dotless operator call
        if call.call_operator_loc().is_some() {
            return false;
        }
        let name = call.name().as_slice();
        // [] and []= subscript operators
        if name == b"[]" || name == b"[]=" {
            return true;
        }
        // Binary/unary operator methods (called without dot)
        matches!(
            name,
            b"+" | b"-"
                | b"*"
                | b"/"
                | b"%"
                | b"**"
                | b"==="
                | b"=="
                | b"!="
                | b"=~"
                | b"!~"
                | b"<"
                | b">"
                | b"<="
                | b">="
                | b"<=>"
                | b"<<"
                | b">>"
                | b"&"
                | b"|"
                | b"^"
                | b"~"
                | b"!"
                | b"+@"
                | b"-@"
        )
    }

    /// Check if a single method name is inherently unsafe for safe navigation.
    fn is_unsafe_single_method(name_bytes: &[u8]) -> bool {
        // empty? — nil&.empty? returns nil, not false, changing behavior
        if name_bytes == b"empty?" {
            return true;
        }
        // Assignment methods
        if name_bytes.ends_with(b"=") && !name_bytes.ends_with(b"==") {
            return true;
        }
        false
    }

    /// Get the single statement from a StatementsNode, if exactly one.
    fn single_stmt_from_stmts<'a>(
        stmts: &ruby_prism::StatementsNode<'a>,
    ) -> Option<ruby_prism::Node<'a>> {
        let body: Vec<_> = stmts.body().iter().collect();
        if body.len() == 1 {
            Some(body.into_iter().next().unwrap())
        } else {
            None
        }
    }

    /// Check if a node is a nil literal.
    fn is_nil(node: &ruby_prism::Node<'_>) -> bool {
        node.as_nil_node().is_some()
    }

    fn node_source<'a>(node: &ruby_prism::Node<'_>, bytes: &'a [u8]) -> &'a [u8] {
        let loc = node.location();
        &bytes[loc.start_offset()..loc.end_offset()]
    }

    fn arguments_source<'a>(
        arguments: &ruby_prism::ArgumentsNode<'_>,
        bytes: &'a [u8],
    ) -> &'a [u8] {
        let loc = arguments.location();
        &bytes[loc.start_offset()..loc.end_offset()]
    }

    fn unwrapped_parenthesized_node<'a>(
        node: &ruby_prism::Node<'a>,
    ) -> Option<ruby_prism::Node<'a>> {
        let parentheses = node.as_parentheses_node()?;
        let body = parentheses.body()?;
        let stmts = body.as_statements_node()?;
        Self::single_stmt_from_stmts(&stmts)
    }

    fn nodes_match(
        left: &ruby_prism::Node<'_>,
        right: &ruby_prism::Node<'_>,
        bytes: &[u8],
    ) -> bool {
        if Self::node_source(left, bytes) == Self::node_source(right, bytes) {
            return true;
        }

        if let Some(inner) = Self::unwrapped_parenthesized_node(left) {
            return Self::nodes_match(&inner, right, bytes);
        }

        // Don't unwrap right side — RuboCop's find_matching_receiver_invocation
        // doesn't unwrap parenthesized conditions, so `obj.bar if (obj)` should
        // not match.

        let (Some(left_call), Some(right_call)) = (left.as_call_node(), right.as_call_node())
        else {
            return false;
        };

        if left_call.name().as_slice() != right_call.name().as_slice() {
            return false;
        }

        let receivers_match = match (left_call.receiver(), right_call.receiver()) {
            (Some(left_receiver), Some(right_receiver)) => {
                Self::nodes_match(&left_receiver, &right_receiver, bytes)
            }
            (None, None) => true,
            _ => false,
        };

        if !receivers_match {
            return false;
        }

        let arguments_match = match (left_call.arguments(), right_call.arguments()) {
            (Some(left_arguments), Some(right_arguments)) => {
                Self::arguments_source(&left_arguments, bytes)
                    == Self::arguments_source(&right_arguments, bytes)
            }
            (None, None) => true,
            _ => false,
        };

        if !arguments_match {
            return false;
        }

        match (left_call.block(), right_call.block()) {
            (Some(left_block), Some(right_block)) => {
                Self::node_source(&left_block, bytes) == Self::node_source(&right_block, bytes)
            }
            (None, None) => true,
            _ => false,
        }
    }

    fn call_chain_from_checked_receiver<'a>(
        node: &ruby_prism::Node<'a>,
        checked_node: &ruby_prism::Node<'a>,
        bytes: &[u8],
    ) -> Option<Vec<ruby_prism::CallNode<'a>>> {
        if let Some(inner) = Self::unwrapped_parenthesized_node(node) {
            return Self::call_chain_from_checked_receiver(&inner, checked_node, bytes);
        }

        let call = node.as_call_node()?;
        let receiver = call.receiver()?;

        if Self::nodes_match(&receiver, checked_node, bytes) {
            return Some(vec![call]);
        }

        if receiver.as_parentheses_node().is_some() {
            return None;
        }

        let mut chain = Self::call_chain_from_checked_receiver(&receiver, checked_node, bytes)?;
        chain.push(call);
        Some(chain)
    }

    /// Search the RHS expression for a call chain whose receiver matches
    /// `checked_node`.  When `flatten_and` is true, inner `and` nodes (with
    /// matching operator) are entered and their left branch is searched —
    /// this mirrors the clause-flattening that RuboCop's `collect_and_clauses`
    /// does at the top level.  Inside blocks, `flatten_and` must be false
    /// because RuboCop's `concat_nodes` skips `and` nodes with a `:block`
    /// ancestor.
    fn first_safe_chain_in_expression<'a>(
        node: &ruby_prism::Node<'a>,
        checked_node: &ruby_prism::Node<'a>,
        bytes: &[u8],
        outer_operator: AndOperatorKind,
        max_chain_length: usize,
        allowed_methods: &Option<Vec<String>>,
        local_unsafe_parent_depth: usize,
        flatten_and: bool,
    ) -> SafeChainSearchResult<'a> {
        if let Some(inner) = Self::unwrapped_parenthesized_node(node) {
            return Self::first_safe_chain_in_expression(
                &inner,
                checked_node,
                bytes,
                outer_operator,
                max_chain_length,
                allowed_methods,
                local_unsafe_parent_depth,
                flatten_and,
            );
        }

        if let Some(call) = node.as_call_node() {
            let Some(chain) =
                Self::call_chain_from_checked_receiver(&call.as_node(), checked_node, bytes)
            else {
                if call.name().as_slice() == b"!" {
                    if let Some(receiver) = call.receiver() {
                        return Self::first_safe_chain_in_nested_and_descendants(
                            &receiver,
                            checked_node,
                            bytes,
                            outer_operator,
                            max_chain_length,
                            allowed_methods,
                            local_unsafe_parent_depth + 1,
                            flatten_and,
                        );
                    }
                }
                return SafeChainSearchResult::NoneFound;
            };

            if local_unsafe_parent_depth > 0 && chain.len() == 1 {
                return SafeChainSearchResult::UnsafeMatch;
            }

            if chain.len() > max_chain_length
                || Self::chain_has_dotless_operator(&chain)
                || Self::has_unsafe_method_after_checked_receiver(&chain, allowed_methods)
            {
                return SafeChainSearchResult::UnsafeMatch;
            }

            return SafeChainSearchResult::Safe(chain);
        }

        if flatten_and {
            if let Some(and) = node.as_and_node() {
                if Self::and_operator_kind(&and, bytes) != outer_operator {
                    return SafeChainSearchResult::NoneFound;
                }

                return Self::first_safe_chain_in_expression(
                    &and.left(),
                    checked_node,
                    bytes,
                    outer_operator,
                    max_chain_length,
                    allowed_methods,
                    local_unsafe_parent_depth,
                    flatten_and,
                );
            }
        }

        if let Some(or) = node.as_or_node() {
            let left_result = Self::first_safe_chain_in_nested_and_descendants(
                &or.left(),
                checked_node,
                bytes,
                outer_operator,
                max_chain_length,
                allowed_methods,
                local_unsafe_parent_depth,
                flatten_and,
            );

            return match left_result {
                SafeChainSearchResult::NoneFound => {
                    Self::first_safe_chain_in_nested_and_descendants(
                        &or.right(),
                        checked_node,
                        bytes,
                        outer_operator,
                        max_chain_length,
                        allowed_methods,
                        local_unsafe_parent_depth,
                        flatten_and,
                    )
                }
                result => result,
            };
        }

        SafeChainSearchResult::NoneFound
    }

    fn first_safe_chain_in_nested_and_descendants<'a>(
        node: &ruby_prism::Node<'a>,
        checked_node: &ruby_prism::Node<'a>,
        bytes: &[u8],
        outer_operator: AndOperatorKind,
        max_chain_length: usize,
        allowed_methods: &Option<Vec<String>>,
        local_unsafe_parent_depth: usize,
        flatten_and: bool,
    ) -> SafeChainSearchResult<'a> {
        if let Some(inner) = Self::unwrapped_parenthesized_node(node) {
            return Self::first_safe_chain_in_nested_and_descendants(
                &inner,
                checked_node,
                bytes,
                outer_operator,
                max_chain_length,
                allowed_methods,
                local_unsafe_parent_depth,
                flatten_and,
            );
        }

        if flatten_and {
            if let Some(and) = node.as_and_node() {
                if Self::and_operator_kind(&and, bytes) != outer_operator {
                    return SafeChainSearchResult::NoneFound;
                }

                return Self::first_safe_chain_in_expression(
                    &and.as_node(),
                    checked_node,
                    bytes,
                    outer_operator,
                    max_chain_length,
                    allowed_methods,
                    local_unsafe_parent_depth,
                    flatten_and,
                );
            }
        }

        if let Some(or) = node.as_or_node() {
            let left_result = Self::first_safe_chain_in_nested_and_descendants(
                &or.left(),
                checked_node,
                bytes,
                outer_operator,
                max_chain_length,
                allowed_methods,
                local_unsafe_parent_depth,
                flatten_and,
            );

            return match left_result {
                SafeChainSearchResult::NoneFound => {
                    Self::first_safe_chain_in_nested_and_descendants(
                        &or.right(),
                        checked_node,
                        bytes,
                        outer_operator,
                        max_chain_length,
                        allowed_methods,
                        local_unsafe_parent_depth,
                        flatten_and,
                    )
                }
                result => result,
            };
        }

        SafeChainSearchResult::NoneFound
    }

    fn has_unsafe_method_after_checked_receiver(
        chain: &[ruby_prism::CallNode<'_>],
        allowed_methods: &Option<Vec<String>>,
    ) -> bool {
        for (index, call) in chain.iter().enumerate() {
            let name_bytes = call.name().as_slice();

            if Self::is_unsafe_single_method(name_bytes) {
                return true;
            }

            if index == 0 {
                continue;
            }

            if NIL_METHODS.contains(&name_bytes) {
                return true;
            }

            if let Some(allowed) = allowed_methods {
                if let Ok(name_str) = std::str::from_utf8(name_bytes) {
                    if allowed.iter().any(|method| method == name_str) {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn chain_has_dotless_operator(chain: &[ruby_prism::CallNode<'_>]) -> bool {
        chain.iter().any(Self::is_dotless_operator)
    }

    fn is_double_colon_call(call: &ruby_prism::CallNode<'_>) -> bool {
        call.call_operator_loc()
            .is_some_and(|operator| operator.as_slice() == b"::")
    }

    fn collect_and_clauses<'a>(
        node: ruby_prism::Node<'a>,
        bytes: &[u8],
        expected_operator: Option<AndOperatorKind>,
        clauses: &mut Vec<ruby_prism::Node<'a>>,
    ) {
        if let Some(parentheses) = node.as_parentheses_node() {
            if let Some(body) = parentheses.body() {
                if let Some(stmts) = body.as_statements_node() {
                    if let Some(inner) = Self::single_stmt_from_stmts(&stmts) {
                        if let Some(and) = inner.as_and_node() {
                            let operator = Self::and_operator_kind(&and, bytes);
                            if expected_operator.is_none_or(|expected| expected == operator) {
                                Self::collect_and_clauses(
                                    and.left(),
                                    bytes,
                                    Some(operator),
                                    clauses,
                                );
                                Self::collect_and_clauses(
                                    and.right(),
                                    bytes,
                                    Some(operator),
                                    clauses,
                                );
                                return;
                            }
                        }
                    }
                }
            }
        }

        if let Some(and) = node.as_and_node() {
            let operator = Self::and_operator_kind(&and, bytes);
            if expected_operator.is_none_or(|expected| expected == operator) {
                Self::collect_and_clauses(and.left(), bytes, Some(operator), clauses);
                Self::collect_and_clauses(and.right(), bytes, Some(operator), clauses);
                return;
            }
        }

        clauses.push(node);
    }

    fn and_operator_kind(node: &ruby_prism::AndNode<'_>, bytes: &[u8]) -> AndOperatorKind {
        let left_loc = node.left().location();
        let right_loc = node.right().location();
        let between = &bytes[left_loc.end_offset()..right_loc.start_offset()];

        if between.windows(2).any(|window| window == b"&&") {
            AndOperatorKind::DoubleAmpersand
        } else {
            AndOperatorKind::KeywordAnd
        }
    }

    fn top_level_and_clauses<'a>(
        node: &ruby_prism::AndNode<'a>,
        bytes: &[u8],
    ) -> Vec<ruby_prism::Node<'a>> {
        let mut clauses = Vec::new();
        let operator = Self::and_operator_kind(node, bytes);
        Self::collect_and_clauses(node.as_node(), bytes, Some(operator), &mut clauses);
        clauses
    }
}

impl Cop for SafeNavigation {
    fn name(&self) -> &'static str {
        "Style/SafeNavigation"
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::cop::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let max_chain_length = config.get_usize("MaxChainLength", 2);
        let _convert_nil = config.get_bool("ConvertCodeThatCanStartToReturnNil", false);
        let allowed_methods = config
            .get_string_array("AllowedMethods")
            .or_else(|| Some(vec!["present?".to_string(), "blank?".to_string()]));

        let mut visitor = SafeNavVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            max_chain_length,
            allowed_methods,
            in_unsafe_parent: 0,
            in_nil_safe_call_ancestor: 0,
            in_ternary_operator_parent: 0,
            in_assignment_or_operator_parent: 0,
            dotted_assignment_parent_starts: Vec::new(),
            in_call_arguments: 0,
            in_dotless_call_arguments: 0,
            in_block_argument: 0,
            in_block: 0,
            direct_receiver_block_bodies: Vec::new(),
            in_dynamic_send_args: 0,
            in_double_colon_call_arguments: 0,
            in_and_clause_visit: 0,
            in_definition_argument_wrapper: 0,
            in_nil_method_call_arguments: 0,
            outer_unsafe_parent_above_current_block_stack: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

/// Visitor that tracks ancestor call context that makes a safe-navigation rewrite unsafe
/// or non-idiomatic: assignment/operator parents, dynamic send arguments, and cases where
/// a candidate expression is used as another call's receiver or argument subtree.
struct SafeNavVisitor<'a> {
    cop: &'a SafeNavigation,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    max_chain_length: usize,
    allowed_methods: Option<Vec<String>>,
    in_unsafe_parent: usize,
    in_nil_safe_call_ancestor: usize,
    in_ternary_operator_parent: usize,
    in_assignment_or_operator_parent: usize,
    dotted_assignment_parent_starts: Vec<usize>,
    in_call_arguments: usize,
    in_dotless_call_arguments: usize,
    in_block_argument: usize,
    in_block: usize,
    direct_receiver_block_bodies: Vec<(usize, usize)>,
    in_dynamic_send_args: usize,
    in_double_colon_call_arguments: usize,
    in_and_clause_visit: usize,
    in_definition_argument_wrapper: usize,
    in_nil_method_call_arguments: usize,
    outer_unsafe_parent_above_current_block_stack: Vec<bool>,
}

impl<'a> SafeNavVisitor<'a> {
    /// Reset operator-level context when entering multi-branch if/unless/elsif.
    /// `in_unsafe_parent` is intentionally NOT reset: RuboCop's `unsafe_method_used?`
    /// ancestor walk crosses if-node boundaries, so an unsafe parent above the if
    /// (e.g. `cookies[k] = if v && v.to_s; ...; else; ...; end`) still suppresses
    /// chain-length-1 offenses inside the branches.
    fn with_reset_parent_operator_context<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved_in_ternary_operator_parent = self.in_ternary_operator_parent;
        let saved_in_assignment_or_operator_parent = self.in_assignment_or_operator_parent;
        let saved_dotted_assignment_parent_starts =
            std::mem::take(&mut self.dotted_assignment_parent_starts);

        self.in_ternary_operator_parent = 0;
        self.in_assignment_or_operator_parent = 0;

        let result = f(self);

        self.in_assignment_or_operator_parent = saved_in_assignment_or_operator_parent;
        self.in_ternary_operator_parent = saved_in_ternary_operator_parent;
        self.dotted_assignment_parent_starts = saved_dotted_assignment_parent_starts;

        result
    }

    fn visit_flattened_and_clauses<'pr>(&mut self, node: &ruby_prism::AndNode<'pr>) {
        let bytes = self.source.as_bytes();
        for clause in SafeNavigation::top_level_and_clauses(node, bytes) {
            self.in_and_clause_visit += 1;
            self.visit(&clause);
            self.in_and_clause_visit -= 1;
        }
    }

    fn visit_direct_and_node<'pr>(&mut self, node: &ruby_prism::AndNode<'pr>) {
        let lhs = node.left();
        let rhs = node.right();

        if lhs.as_parentheses_node().is_some() {
            ruby_prism::visit_and_node(self, node);
            return;
        }
        let bytes = self.source.as_bytes();
        let operator = SafeNavigation::and_operator_kind(node, bytes);
        let chain = match SafeNavigation::first_safe_chain_in_expression(
            &rhs,
            &lhs,
            bytes,
            operator,
            self.max_chain_length,
            &self.allowed_methods,
            0,
            false, // block context: don't flatten inner `and` nodes
        ) {
            SafeChainSearchResult::Safe(chain) => chain,
            _ => {
                ruby_prism::visit_and_node(self, node);
                return;
            }
        };

        if self.is_direct_receiver_block_body(&node.as_node())
            && chain.iter().any(|call| call.block().is_some())
            && chain.len() >= self.max_chain_length
        {
            ruby_prism::visit_and_node(self, node);
            return;
        }

        if self.in_dynamic_send_args > 0 && chain.len() == 1 {
            ruby_prism::visit_and_node(self, node);
            return;
        }

        // Match RuboCop's nil-method ancestor walk: chain-length-1 can escape to
        // outer `==`, `instance_of?`, etc., while chain-length >= 2 is bounded by
        // the safe-navigation chain itself.
        if self.in_nil_safe_call_ancestor > 0 && chain.len() == 1 {
            ruby_prism::visit_and_node(self, node);
            return;
        }

        // RuboCop's `unsafe_method_used?` walks send ancestors from the first
        // call in the chain up to the rhs expression.  For chain length 1 the
        // walk escapes the expression and finds unsafe (dotless / operator /
        // assignment) parents above.  For chain length ≥ 2 the walk is normally
        // bounded by the outermost call in the chain — but if that call has a
        // block, the block node is not a :call type so the walk escapes past it
        // and finds the unsafe parent.
        if self.in_unsafe_parent > 0
            && (chain.len() == 1 || chain.last().is_some_and(|c| c.block().is_some()))
        {
            ruby_prism::visit_and_node(self, node);
            return;
        }

        let loc = node.location();
        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Use safe navigation (`&.`) instead of checking if an object exists before calling the method.".to_string(),
        ));
    }

    fn is_assignment_or_operator_parent_call(call: &ruby_prism::CallNode<'_>) -> bool {
        let name = call.name().as_slice();

        if name.ends_with(b"=") && name != b"==" && name != b"!=" {
            return true;
        }

        call.call_operator_loc().is_none()
            && matches!(
                name,
                b"[]"
                    | b"+"
                    | b"-"
                    | b"*"
                    | b"/"
                    | b"%"
                    | b"**"
                    | b"==="
                    | b"=="
                    | b"!="
                    | b"=~"
                    | b"!~"
                    | b"<"
                    | b">"
                    | b"<="
                    | b">="
                    | b"<=>"
                    | b"<<"
                    | b">>"
                    | b"&"
                    | b"|"
                    | b"^"
                    | b"~"
                    | b"!"
                    | b"+@"
                    | b"-@"
            )
    }

    /// Check if a call node is an unsafe parent context for safe navigation.
    /// This means the call is an assignment method (name ends with `=`) or
    /// a dotless operator call.
    fn is_unsafe_parent_call(call: &ruby_prism::CallNode<'_>) -> bool {
        let name = call.name().as_slice();
        // Negation wrappers like `!(foo && foo.bar)` or `!!(...)` are unsafe
        // because converting the inner guard to `&.` changes the boolean result.
        if name == b"!" {
            return true;
        }
        // Assignment methods: []=, foo=, etc. (but not == or !=)
        if name.ends_with(b"=") && name != b"==" && name != b"!=" {
            return true;
        }
        // Dotless calls (no dot/safe-nav operator)
        if call.call_operator_loc().is_none() {
            // Binary/unary operator methods
            if matches!(
                name,
                b"+" | b"-"
                    | b"*"
                    | b"/"
                    | b"%"
                    | b"**"
                    | b"==="
                    | b"=~"
                    | b"!~"
                    | b"<"
                    | b">"
                    | b"<="
                    | b">="
                    | b"<=>"
                    | b"<<"
                    | b">>"
                    | b"&"
                    | b"|"
                    | b"^"
            ) {
                return true;
            }
            // Dotless method calls with arguments or a block (e.g., `scope :bar, lambda`,
            // `puts(foo && foo.bar)`). RuboCop considers these unsafe because converting
            // `&&` to safe navigation inside their arguments changes behavior.
            // Excludes: `!` (negation) and bare names with no args (variable-like reads).
            if name != b"!" && (call.arguments().is_some() || call.block().is_some()) {
                return true;
            }
        }
        false
    }

    fn is_dynamic_send_call(call: &ruby_prism::CallNode<'_>) -> bool {
        matches!(
            call.name().as_slice(),
            b"send" | b"__send__" | b"public_send"
        )
    }

    fn is_ternary_operator_parent_call(call: &ruby_prism::CallNode<'_>) -> bool {
        let name = call.name().as_slice();

        if name == b"!" {
            return true;
        }

        call.call_operator_loc().is_none()
            && matches!(
                name,
                b"+" | b"-"
                    | b"*"
                    | b"/"
                    | b"%"
                    | b"**"
                    | b"==="
                    | b"=="
                    | b"!="
                    | b"=~"
                    | b"!~"
                    | b"<"
                    | b">"
                    | b"<="
                    | b">="
                    | b"<=>"
                    | b"<<"
                    | b">>"
                    | b"&"
                    | b"|"
                    | b"^"
                    | b"~"
                    | b"+@"
                    | b"-@"
            )
    }

    fn is_dotted_assignment_parent_call(call: &ruby_prism::CallNode<'_>) -> bool {
        let name = call.name().as_slice();
        call.call_operator_loc().is_some() && name.ends_with(b"=") && name != b"==" && name != b"!="
    }

    fn is_nil_safe_call_ancestor(call: &ruby_prism::CallNode<'_>) -> bool {
        NIL_METHODS.contains(&call.name().as_slice())
    }

    fn collect_direct_receiver_block_bodies(
        node: &ruby_prism::Node<'_>,
        bodies: &mut Vec<(usize, usize)>,
    ) {
        if let Some(parentheses) = node.as_parentheses_node() {
            if let Some(body) = parentheses.body() {
                if let Some(stmts) = body.as_statements_node() {
                    if let Some(inner) = SafeNavigation::single_stmt_from_stmts(&stmts) {
                        Self::collect_direct_receiver_block_bodies(&inner, bodies);
                    }
                }
            }
            return;
        }

        let block = if let Some(call) = node.as_call_node() {
            call.block().and_then(|block| block.as_block_node())
        } else {
            node.as_block_node()
        };
        let Some(block) = block else {
            return;
        };
        let Some(body) = block.body() else {
            return;
        };
        let Some(stmts) = body.as_statements_node() else {
            return;
        };
        let loc = stmts.location();
        bodies.push((loc.start_offset(), loc.end_offset()));
    }

    fn is_direct_receiver_block_body(&self, node: &ruby_prism::Node<'_>) -> bool {
        let loc = node.location();
        let range = (loc.start_offset(), loc.end_offset());
        self.direct_receiver_block_bodies.contains(&range)
            || self
                .direct_receiver_block_bodies
                .iter()
                .any(|(start, end)| range.0 >= *start && range.1 <= *end)
    }
}

impl<'a, 'pr> Visit<'pr> for SafeNavVisitor<'a> {
    fn visit_block_argument_node(&mut self, node: &ruby_prism::BlockArgumentNode<'pr>) {
        self.in_block_argument += 1;
        ruby_prism::visit_block_argument_node(self, node);
        self.in_block_argument -= 1;
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        self.in_block += 1;
        if self.in_call_arguments > 0 && self.in_definition_argument_wrapper == 0 {
            // Reset call-argument and operator contexts for the block body, but
            // preserve `in_unsafe_parent`.  RuboCop's `unsafe_method_used?` walks
            // all send ancestors (crossing block boundaries) and finds dotless
            // parents above the block.  For `&&` patterns the chain-length check
            // in `visit_and_node` / `visit_direct_and_node` decides whether the
            // unsafe parent actually suppresses the offense.
            let saved_in_call_arguments = self.in_call_arguments;
            self.in_call_arguments = 0;
            let saved_in_dotless_call_arguments = self.in_dotless_call_arguments;
            self.in_dotless_call_arguments = 0;
            let saved_in_ternary_operator_parent = self.in_ternary_operator_parent;
            let saved_in_assignment_or_operator_parent = self.in_assignment_or_operator_parent;
            let saved_dotted = std::mem::take(&mut self.dotted_assignment_parent_starts);
            self.in_ternary_operator_parent = 0;
            self.in_assignment_or_operator_parent = 0;

            ruby_prism::visit_block_node(self, node);

            self.in_assignment_or_operator_parent = saved_in_assignment_or_operator_parent;
            self.in_ternary_operator_parent = saved_in_ternary_operator_parent;
            self.dotted_assignment_parent_starts = saved_dotted;
            self.in_call_arguments = saved_in_call_arguments;
            self.in_dotless_call_arguments = saved_in_dotless_call_arguments;
        } else {
            ruby_prism::visit_block_node(self, node);
        }
        self.in_block -= 1;
    }

    fn visit_assoc_node(&mut self, node: &ruby_prism::AssocNode<'pr>) {
        self.visit(&node.key());
        // Reset operator-level context for hash values, but preserve in_unsafe_parent
        // so that `&&` inside hash values of dotless calls remains suppressed.
        // RuboCop walks all send ancestors, so a dotless call like `before_save`
        // suppresses `&&` even inside hash-value lambdas. Modifier-if/unless
        // should NOT inherit dotless-argument suppression through hash pairs,
        // so reset `in_dotless_call_arguments` while visiting the value.
        let saved_ternary = self.in_ternary_operator_parent;
        let saved_assignment = self.in_assignment_or_operator_parent;
        let saved_dotted = std::mem::take(&mut self.dotted_assignment_parent_starts);
        let saved_dotless_call_arguments = self.in_dotless_call_arguments;
        self.in_ternary_operator_parent = 0;
        self.in_assignment_or_operator_parent = 0;
        self.in_dotless_call_arguments = 0;
        self.visit(&node.value());
        self.in_ternary_operator_parent = saved_ternary;
        self.in_assignment_or_operator_parent = saved_assignment;
        self.dotted_assignment_parent_starts = saved_dotted;
        self.in_dotless_call_arguments = saved_dotless_call_arguments;
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        // Lambdas (`-> { ... }`) behave like blocks in RuboCop's AST: `&&` inside
        // lambdas with block ancestors is skipped by `concat_nodes`. Reset
        // `in_call_arguments` so modifier-if/ternary checks inside lambdas are not
        // suppressed by the outer call context, while preserving `in_unsafe_parent`
        // so that `&&` patterns remain correctly suppressed by dotless ancestors.
        self.in_block += 1;
        if self.in_call_arguments > 0 {
            let saved_call_args = self.in_call_arguments;
            let saved_nil_method = self.in_nil_method_call_arguments;
            let saved_dotless = self.in_dotless_call_arguments;
            self.in_call_arguments = 0;
            self.in_nil_method_call_arguments = 0;
            self.in_dotless_call_arguments = 0;
            ruby_prism::visit_lambda_node(self, node);
            self.in_call_arguments = saved_call_args;
            self.in_nil_method_call_arguments = saved_nil_method;
            self.in_dotless_call_arguments = saved_dotless;
        } else {
            ruby_prism::visit_lambda_node(self, node);
        }
        self.in_block -= 1;
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        if self.in_call_arguments > 0 {
            self.in_definition_argument_wrapper += 1;
        }
        ruby_prism::visit_def_node(self, node);
        if self.in_call_arguments > 0 {
            self.in_definition_argument_wrapper -= 1;
        }
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        if self.in_call_arguments > 0 {
            self.in_definition_argument_wrapper += 1;
        }
        ruby_prism::visit_class_node(self, node);
        if self.in_call_arguments > 0 {
            self.in_definition_argument_wrapper -= 1;
        }
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        if self.in_call_arguments > 0 {
            self.in_definition_argument_wrapper += 1;
        }
        ruby_prism::visit_module_node(self, node);
        if self.in_call_arguments > 0 {
            self.in_definition_argument_wrapper -= 1;
        }
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        if self.in_call_arguments > 0 {
            self.in_definition_argument_wrapper += 1;
        }
        ruby_prism::visit_singleton_class_node(self, node);
        if self.in_call_arguments > 0 {
            self.in_definition_argument_wrapper -= 1;
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let is_unsafe = Self::is_unsafe_parent_call(node);
        if is_unsafe {
            self.in_unsafe_parent += 1;
        }
        let is_nil_safe_call_ancestor = Self::is_nil_safe_call_ancestor(node);
        let is_assignment_or_operator_parent = Self::is_assignment_or_operator_parent_call(node);
        if is_assignment_or_operator_parent {
            self.in_assignment_or_operator_parent += 1;
        }
        let is_ternary_operator_parent = Self::is_ternary_operator_parent_call(node);
        if is_ternary_operator_parent {
            self.in_ternary_operator_parent += 1;
        }
        let is_dotted_assignment_parent = Self::is_dotted_assignment_parent_call(node);
        if is_dotted_assignment_parent {
            self.dotted_assignment_parent_starts
                .push(node.location().start_offset());
        }
        if let Some(receiver) = node.receiver() {
            let receiver_block_bodies_len = self.direct_receiver_block_bodies.len();
            if is_nil_safe_call_ancestor {
                self.in_nil_safe_call_ancestor += 1;
            }
            Self::collect_direct_receiver_block_bodies(
                &receiver,
                &mut self.direct_receiver_block_bodies,
            );
            self.visit(&receiver);
            if is_nil_safe_call_ancestor {
                self.in_nil_safe_call_ancestor -= 1;
            }
            self.direct_receiver_block_bodies
                .truncate(receiver_block_bodies_len);
        }

        if let Some(arguments) = node.arguments() {
            self.in_call_arguments += 1;
            let saved_in_dotless_call_arguments = self.in_dotless_call_arguments;
            // Track only dotless non-operator calls like `scope(...)` / `puts(...)`.
            // Dotless operator calls (`[]=`, `+`, etc.) are handled separately via
            // `in_assignment_or_operator_parent` with chain-length-aware suppression.
            if node.call_operator_loc().is_none() && !SafeNavigation::is_dotless_operator(node) {
                self.in_dotless_call_arguments = self.in_call_arguments;
            }
            let is_dynamic_send = Self::is_dynamic_send_call(node);
            let is_double_colon = SafeNavigation::is_double_colon_call(node);
            let is_nil_method_args = is_nil_safe_call_ancestor;
            if is_dynamic_send {
                self.in_dynamic_send_args += 1;
            }
            if is_double_colon {
                self.in_double_colon_call_arguments += 1;
            }
            if is_nil_method_args {
                self.in_nil_method_call_arguments += 1;
            }
            self.visit_arguments_node(&arguments);
            if is_nil_method_args {
                self.in_nil_method_call_arguments -= 1;
            }
            if is_dynamic_send {
                self.in_dynamic_send_args -= 1;
            }
            if is_double_colon {
                self.in_double_colon_call_arguments -= 1;
            }
            self.in_call_arguments -= 1;
            self.in_dotless_call_arguments = saved_in_dotless_call_arguments;
        }

        if let Some(block) = node.block() {
            if is_unsafe && self.in_and_clause_visit == 0 {
                self.in_unsafe_parent -= 1;
            }
            self.outer_unsafe_parent_above_current_block_stack
                .push(self.in_unsafe_parent > 0);
            self.visit(&block);
            self.outer_unsafe_parent_above_current_block_stack.pop();
        }

        if is_assignment_or_operator_parent {
            self.in_assignment_or_operator_parent -= 1;
        }
        if is_ternary_operator_parent {
            self.in_ternary_operator_parent -= 1;
        }
        if is_dotted_assignment_parent {
            self.dotted_assignment_parent_starts.pop();
        }
        if is_unsafe && node.block().is_none_or(|_| self.in_and_clause_visit > 0) {
            self.in_unsafe_parent -= 1;
        }
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        if self.in_block_argument > 0 && self.in_block == 0 {
            self.visit_flattened_and_clauses(node);
            return;
        }

        if self.in_double_colon_call_arguments > 0 {
            self.visit_flattened_and_clauses(node);
            return;
        }

        if self.in_block > 0 {
            self.visit_direct_and_node(node);
            return;
        }

        let bytes = self.source.as_bytes();
        let clauses = SafeNavigation::top_level_and_clauses(node, bytes);
        let operator = SafeNavigation::and_operator_kind(node, bytes);

        for pair in clauses.windows(2) {
            let lhs = &pair[0];
            let rhs = &pair[1];

            if lhs.as_parentheses_node().is_some() {
                continue;
            }

            let chain = match SafeNavigation::first_safe_chain_in_expression(
                rhs,
                lhs,
                bytes,
                operator,
                self.max_chain_length,
                &self.allowed_methods,
                0,
                true, // top-level: flatten inner `and` nodes like RuboCop's collect_and_clauses
            ) {
                SafeChainSearchResult::Safe(chain) => chain,
                _ => continue,
            };

            // RuboCop's `chain_length` uses `each_ancestor(:call)` which
            // leaks past block nodes (not `:call` type).  In a direct-receiver
            // block body the leaked chain length includes the outer receiver
            // chain, pushing the total over `max_chain_length` — but only
            // when the chain is already at max length (chain_length + at_least_1
            // outer call > max).  Chain length 1 still fits: 1 + 1 = 2 ≤ 2.
            if self.is_direct_receiver_block_body(&node.as_node())
                && chain.iter().any(|call| call.block().is_some())
                && chain.len() >= self.max_chain_length
            {
                continue;
            }

            if self.in_dynamic_send_args > 0 && chain.len() == 1 {
                continue;
            }

            // Match RuboCop's nil-method ancestor walk: chain-length-1 can
            // escape to outer nil methods, while chain-length >= 2 stops at the
            // outermost call in the safe-navigation chain.
            if self.in_nil_safe_call_ancestor > 0 && chain.len() == 1 {
                continue;
            }

            // RuboCop's `unsafe_method_used?` ancestor walk is bounded by the
            // rhs expression for chain length ≥ 2 but escapes for length 1.
            // It also escapes for chain length ≥ 2 when the outermost call has
            // a block, because the block node is not a :call type.
            if self.in_unsafe_parent > 0
                && (chain.len() == 1 || chain.last().is_some_and(|c| c.block().is_some()))
            {
                continue;
            }

            let loc = lhs.location();
            let (line, column) = self.source.offset_to_line_col(loc.start_offset());
            self.diagnostics.push(self.cop.diagnostic(
                self.source,
                line,
                column,
                "Use safe navigation (`&.`) instead of checking if an object exists before calling the method.".to_string(),
            ));
        }

        // Always visit flattened clauses so that inner `&&` expressions
        // (e.g. inside parenthesized `or` branches) are checked even when
        // the top-level chain already produced an offense.  RuboCop's
        // `on_and` fires independently for each `and` node in the AST, so
        // nested `and` nodes get their own `on_and` call regardless of
        // whether an ancestor `and` was an offense.
        self.visit_flattened_and_clauses(node);
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        let if_node = node;

        // Check if it's a ternary (no `if` keyword location in Prism)
        if if_node.if_keyword_loc().is_none() {
            // RuboCop's `unsafe_method?` always returns false for ternary nodes
            // (except negation), so ternaries are NOT suppressed by dotless/unsafe
            // call ancestors.  They ARE suppressed when a nil-responding method
            // (like `==`, `!`, `is_a?`) is an ancestor — either as receiver
            // (`in_nil_safe_call_ancestor`) or as arguments (`in_nil_method_call_arguments`).
            // `NIL_METHODS` includes operators like `==`, `!`, `&`, `|`, `^` which
            // nil responds to, so `(x ? x.y : nil) == z` is correctly suppressed.
            if (self.in_block_argument > 0 && self.in_block == 0)
                || self.in_nil_safe_call_ancestor > 0
                || self.in_nil_method_call_arguments > 0
            {
                ruby_prism::visit_if_node(self, node);
                return;
            }

            let diags = self.cop.check_ternary(
                self.source,
                if_node,
                TernaryCheckContext {
                    max_chain_length: self.max_chain_length,
                    allowed_methods: &self.allowed_methods,
                    offense_start_offset: self.dotted_assignment_parent_starts.last().copied(),
                    skip_nested_block_call_args: self.in_call_arguments > 1,
                    skip_direct_receiver_block_body_block_calls: self
                        .is_direct_receiver_block_body(&node.as_node()),
                },
            );
            self.diagnostics.extend(diags);
            ruby_prism::visit_if_node(self, node);
            return;
        }

        let node_loc = if_node.location();

        // Check modifier if patterns: `foo.bar if foo`
        let kw = if_node.if_keyword_loc().unwrap();
        let is_unless = kw.as_slice() == b"unless";

        // Skip elsif
        if kw.as_slice() == b"elsif" {
            self.with_reset_parent_operator_context(|this| {
                ruby_prism::visit_if_node(this, node);
            });
            return;
        }

        // Must not have else/elsif
        if if_node.subsequent().is_some() {
            self.with_reset_parent_operator_context(|this| {
                ruby_prism::visit_if_node(this, node);
            });
            return;
        }

        if (self.in_block_argument > 0 && self.in_block == 0)
            || self.in_nil_safe_call_ancestor > 0
            || self.in_dotless_call_arguments > 0
            || self.in_dynamic_send_args > 0
        {
            ruby_prism::visit_if_node(self, node);
            return;
        }

        let diags = self.cop.check_modifier_if(
            self.source,
            &node_loc,
            if_node,
            is_unless,
            ModifierIfCheckContext {
                max_chain_length: self.max_chain_length,
                allowed_methods: &self.allowed_methods,
                skip_direct_receiver_block_body_block_calls: self
                    .is_direct_receiver_block_body(&node.as_node()),
                in_unsafe_parent: self.in_unsafe_parent > 0,
                in_assignment_or_operator_parent: self.in_assignment_or_operator_parent > 0,
                has_outer_unsafe_parent_above_current_block: self
                    .outer_unsafe_parent_above_current_block_stack
                    .last()
                    .copied()
                    .unwrap_or(false),
            },
        );
        self.diagnostics.extend(diags);

        ruby_prism::visit_if_node(self, node);
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        // Must not have else
        if node.else_clause().is_some() {
            self.with_reset_parent_operator_context(|this| {
                ruby_prism::visit_unless_node(this, node);
            });
            return;
        }

        if (self.in_block_argument > 0 && self.in_block == 0)
            || self.in_nil_safe_call_ancestor > 0
            || self.in_dotless_call_arguments > 0
            || self.in_dynamic_send_args > 0
        {
            ruby_prism::visit_unless_node(self, node);
            return;
        }

        let node_loc = node.location();
        let condition = node.predicate();
        let body_stmts = match node.statements() {
            Some(s) => s,
            None => {
                ruby_prism::visit_unless_node(self, node);
                return;
            }
        };

        // Must have exactly one body statement
        let body = match SafeNavigation::single_stmt_from_stmts(&body_stmts) {
            Some(n) => n,
            None => {
                ruby_prism::visit_unless_node(self, node);
                return;
            }
        };

        let bytes = self.source.as_bytes();

        // Extract checked_src: `unless foo.nil?` → check foo
        let checked_node: Option<ruby_prism::Node<'_>> =
            if let Some(call) = condition.as_call_node() {
                let name = call.name().as_slice();
                if name == b"nil?" {
                    if method_dispatch_predicates::is_safe_navigation(&call) {
                        ruby_prism::visit_unless_node(self, node);
                        return;
                    }
                    call.receiver()
                } else {
                    None
                }
            } else {
                None
            };

        let checked_node = match checked_node {
            Some(s) => s,
            None => {
                ruby_prism::visit_unless_node(self, node);
                return;
            }
        };

        // Body must be a method call chain
        let body_call = match body.as_call_node() {
            Some(c) => c,
            None => {
                ruby_prism::visit_unless_node(self, node);
                return;
            }
        };

        if body_call.call_operator_loc().is_none() {
            ruby_prism::visit_unless_node(self, node);
            return;
        }

        let chain =
            match SafeNavigation::call_chain_from_checked_receiver(&body, &checked_node, bytes) {
                Some(chain) => chain,
                None => {
                    ruby_prism::visit_unless_node(self, node);
                    return;
                }
            };

        if chain.len() > self.max_chain_length {
            ruby_prism::visit_unless_node(self, node);
            return;
        }

        if SafeNavigation::chain_has_dotless_operator(&chain) {
            ruby_prism::visit_unless_node(self, node);
            return;
        }

        if SafeNavigation::has_unsafe_method_after_checked_receiver(&chain, &self.allowed_methods) {
            ruby_prism::visit_unless_node(self, node);
            return;
        }

        // RuboCop's `unsafe_method_used?` ancestor walk escapes the method chain
        // for chain-length-1 and finds unsafe parents above the unless node.
        if self.in_unsafe_parent > 0 && chain.len() == 1 {
            ruby_prism::visit_unless_node(self, node);
            return;
        }

        if self.is_direct_receiver_block_body(&node.as_node())
            && chain.iter().any(|call| call.block().is_some())
        {
            ruby_prism::visit_unless_node(self, node);
            return;
        }

        let (line, column) = self.source.offset_to_line_col(node_loc.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Use safe navigation (`&.`) instead of checking if an object exists before calling the method.".to_string(),
        ));

        ruby_prism::visit_unless_node(self, node);
    }
}

impl SafeNavigation {
    fn check_ternary(
        &self,
        source: &SourceFile,
        if_node: &ruby_prism::IfNode<'_>,
        context: TernaryCheckContext<'_>,
    ) -> Vec<Diagnostic> {
        let condition = if_node.predicate();
        let bytes = source.as_bytes();

        // Extract checked_variable range and determine which branch is the body
        // Patterns:
        // 1. foo.nil? ? nil : foo.bar  => checked_var = foo, body = else_branch
        // 2. !foo.nil? ? foo.bar : nil => checked_var = foo, body = if_branch
        // 3. foo ? foo.bar : nil       => checked_var = foo, body = if_branch

        // Determine condition type
        let (checked_node, body_is_else) = if let Some(call) = condition.as_call_node() {
            let name = call.name().as_slice();
            if name == b"nil?" {
                if method_dispatch_predicates::is_safe_navigation(&call) {
                    return Vec::new();
                }
                // foo.nil? ? nil : foo.bar
                if let Some(recv) = call.receiver() {
                    // if_branch must be nil
                    let if_is_nil = if_node
                        .statements()
                        .and_then(|s| Self::single_stmt_from_stmts(&s))
                        .is_none_or(|n| Self::is_nil(&n));
                    if !if_is_nil {
                        return Vec::new();
                    }
                    (recv, true) // body is else branch
                } else {
                    return Vec::new();
                }
            } else if name == b"!" {
                // !foo or !foo.nil?
                if let Some(recv) = call.receiver() {
                    if let Some(inner_call) = recv.as_call_node() {
                        if inner_call.name().as_slice() == b"nil?" {
                            if method_dispatch_predicates::is_safe_navigation(&inner_call) {
                                return Vec::new();
                            }
                            // !foo.nil? ? foo.bar : nil
                            if let Some(inner_recv) = inner_call.receiver() {
                                // else_branch must be nil
                                let else_is_nil = self.else_branch_is_nil(if_node);
                                if !else_is_nil {
                                    return Vec::new();
                                }
                                (inner_recv, false) // body is if branch
                            } else {
                                return Vec::new();
                            }
                        } else {
                            // !foo ? nil : foo.bar
                            let if_is_nil = if_node
                                .statements()
                                .and_then(|s| Self::single_stmt_from_stmts(&s))
                                .is_none_or(|n| Self::is_nil(&n));
                            if !if_is_nil {
                                return Vec::new();
                            }
                            (recv, true) // body is else branch
                        }
                    } else {
                        // !foo ? nil : foo.bar
                        let if_is_nil = if_node
                            .statements()
                            .and_then(|s| Self::single_stmt_from_stmts(&s))
                            .is_none_or(|n| Self::is_nil(&n));
                        if !if_is_nil {
                            return Vec::new();
                        }
                        (recv, true)
                    }
                } else {
                    return Vec::new();
                }
            } else {
                // foo ? foo.bar : nil => plain variable/expression check
                // else_branch must be nil
                let else_is_nil = self.else_branch_is_nil(if_node);
                if !else_is_nil {
                    return Vec::new();
                }
                (condition, false) // body is if branch
            }
        } else {
            // Non-call condition: foo ? foo.bar : nil
            let else_is_nil = self.else_branch_is_nil(if_node);
            if !else_is_nil {
                return Vec::new();
            }
            (condition, false)
        };

        // Get the body node (the non-nil branch)
        let body = if body_is_else {
            // Body is in else branch
            let subsequent = match if_node.subsequent() {
                Some(s) => s,
                None => return Vec::new(),
            };
            let else_node = match subsequent.as_else_node() {
                Some(e) => e,
                None => return Vec::new(),
            };
            match else_node
                .statements()
                .and_then(|s| Self::single_stmt_from_stmts(&s))
            {
                Some(n) => n,
                None => return Vec::new(),
            }
        } else {
            // Body is in if branch
            match if_node
                .statements()
                .and_then(|s| Self::single_stmt_from_stmts(&s))
            {
                Some(n) => n,
                None => return Vec::new(),
            }
        };

        // Body must be a method call chain with a dot operator
        let body_call = match body.as_call_node() {
            Some(c) => c,
            None => return Vec::new(),
        };

        if context.skip_nested_block_call_args && body_call.block().is_some() {
            return Vec::new();
        }

        if body_call.call_operator_loc().is_none() {
            return Vec::new();
        }

        // Find matching receiver using source byte comparison
        let chain = match Self::call_chain_from_checked_receiver(&body, &checked_node, bytes) {
            Some(chain) => chain,
            None => return Vec::new(),
        };

        if context.skip_direct_receiver_block_body_block_calls
            && chain.iter().any(|call| call.block().is_some())
        {
            return Vec::new();
        }

        if chain.len() > context.max_chain_length {
            return Vec::new();
        }

        if Self::chain_has_dotless_operator(&chain) {
            return Vec::new();
        }

        if Self::has_unsafe_method_after_checked_receiver(&chain, context.allowed_methods) {
            return Vec::new();
        }

        let node_loc = if_node.location();
        let (line, column) = source.offset_to_line_col(
            context
                .offense_start_offset
                .unwrap_or(node_loc.start_offset()),
        );
        vec![self.diagnostic(
            source,
            line,
            column,
            "Use safe navigation (`&.`) instead of checking if an object exists before calling the method.".to_string(),
        )]
    }

    fn else_branch_is_nil(&self, if_node: &ruby_prism::IfNode<'_>) -> bool {
        match if_node.subsequent() {
            Some(subsequent) => {
                match subsequent.as_else_node() {
                    Some(else_node) => {
                        match else_node.statements() {
                            Some(stmts) => {
                                match Self::single_stmt_from_stmts(&stmts) {
                                    Some(n) => Self::is_nil(&n),
                                    None => true, // empty else => nil
                                }
                            }
                            None => true, // no statements => nil
                        }
                    }
                    None => false,
                }
            }
            None => false, // no else branch at all — not the pattern we want
        }
    }

    fn check_modifier_if(
        &self,
        source: &SourceFile,
        node_loc: &ruby_prism::Location<'_>,
        if_node: &ruby_prism::IfNode<'_>,
        is_unless: bool,
        context: ModifierIfCheckContext<'_>,
    ) -> Vec<Diagnostic> {
        let condition = if_node.predicate();
        let body_stmts = match if_node.statements() {
            Some(s) => s,
            None => return Vec::new(),
        };

        // Must have exactly one body statement
        let body = match Self::single_stmt_from_stmts(&body_stmts) {
            Some(n) => n,
            None => return Vec::new(),
        };

        let bytes = source.as_bytes();

        let condition_is_method_call = Self::is_method_called(&condition);

        // Extract the checked variable from the condition
        let checked_node: Option<ruby_prism::Node<'_>> =
            if let Some(call) = condition.as_call_node() {
                let name = call.name().as_slice();
                if name == b"nil?" {
                    if method_dispatch_predicates::is_safe_navigation(&call) {
                        return Vec::new();
                    }
                    // unless foo.nil? => check foo
                    if is_unless {
                        call.receiver()
                    } else {
                        return Vec::new();
                    }
                } else if name == b"!" {
                    // if !foo or if !foo.nil?
                    call.receiver().and_then(|r| {
                        if let Some(inner) = r.as_call_node() {
                            if inner.name().as_slice() == b"nil?" {
                                if method_dispatch_predicates::is_safe_navigation(&inner) {
                                    return None;
                                }
                                if is_unless { None } else { inner.receiver() }
                            } else if is_unless {
                                Some(r)
                            } else {
                                None
                            }
                        } else if is_unless {
                            Some(r)
                        } else {
                            None
                        }
                    })
                } else {
                    // foo.bar if foo
                    if !is_unless {
                        Some(if_node.predicate())
                    } else {
                        return Vec::new();
                    }
                }
            } else {
                // Plain variable: `foo.bar if foo`
                if !is_unless {
                    Some(if_node.predicate())
                } else {
                    return Vec::new();
                }
            };

        let checked_node = match checked_node {
            Some(s) => s,
            None => return Vec::new(),
        };

        // Body must be a method call chain
        let body_call = match body.as_call_node() {
            Some(c) => c,
            None => return Vec::new(),
        };

        if body_call.call_operator_loc().is_none() {
            return Vec::new();
        }

        let chain = match Self::call_chain_from_checked_receiver(&body, &checked_node, bytes) {
            Some(chain) => chain,
            None => return Vec::new(),
        };

        if context.skip_direct_receiver_block_body_block_calls
            && chain.iter().any(|call| call.block().is_some())
        {
            return Vec::new();
        }

        if body_call.block().is_some()
            && !context.skip_direct_receiver_block_body_block_calls
            && context.has_outer_unsafe_parent_above_current_block
        {
            return Vec::new();
        }

        if chain.len() > context.max_chain_length {
            return Vec::new();
        }

        if Self::chain_has_dotless_operator(&chain) {
            return Vec::new();
        }

        if Self::has_unsafe_method_after_checked_receiver(&chain, context.allowed_methods) {
            return Vec::new();
        }

        // RuboCop's `unsafe_method_used?` ancestor walk escapes the method chain
        // for chain-length-1 and finds unsafe parents (dotless calls, operators, etc.)
        // above the modifier-if.  For chain ≥ 2, the walk is bounded by the chain.
        if context.in_unsafe_parent && chain.len() == 1 {
            return Vec::new();
        }

        if context.in_assignment_or_operator_parent && chain.len() == 1 {
            return Vec::new();
        }

        // RuboCop: use_var_only_in_unless_modifier? — for `unless foo`, skip
        // if the checked variable is used only in the condition (not a method call)
        if is_unless && !condition_is_method_call {
            return Vec::new();
        }

        let (line, column) = source.offset_to_line_col(node_loc.start_offset());
        vec![self.diagnostic(
            source,
            line,
            column,
            "Use safe navigation (`&.`) instead of checking if an object exists before calling the method.".to_string(),
        )]
    }

    /// Check if the condition node is a method call (has a parent send)
    fn is_method_called(node: &ruby_prism::Node<'_>) -> bool {
        // In RuboCop, this checks `send_node&.parent&.send_type?`
        // We approximate: if the condition itself is a call node with a receiver
        if let Some(call) = node.as_call_node() {
            return call.receiver().is_some();
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(SafeNavigation, "cops/style/safe_navigation");
}
