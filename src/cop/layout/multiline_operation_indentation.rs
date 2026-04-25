use std::cell::RefCell;
use std::collections::HashMap;

use ruby_prism::Visit;

use crate::cop::shared::method_identifier_predicates;
use crate::cop::shared::node_type::{AND_NODE, CALL_NODE, OR_NODE};
use crate::cop::shared::util::{
    begins_its_line, indentation_of, is_modifier_if, is_modifier_unless,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Checks indentation of multiline binary operations.
///
/// Key fix (2026-04-01): removed blanket skip for nested boolean chains
/// (And/Or as left operand) which was the root cause of ~39k FN.  Added
/// RuboCop-compatible `begins_its_line?` guard that skips leading-operator
/// patterns like `expr \n  && other_expr`, fixing the confirmed FP class.
/// Tightened the `is_ok` check for And/Or nodes: in non-keyword contexts
/// only `left_indent + width` is accepted (not `left_col`); in keyword
/// conditions with aligned style, alignment with `left_col` or double-width
/// `kw_expected` are accepted.
///
/// Key fix (2026-04-04): CallNode (`+`, `-`, etc.) now delegates to
/// `check_binary_node` with `accept_left_alignment=true`. This adds
/// assignment/keyword context awareness (fixing FN where RuboCop requires
/// alignment but old code accepted wrong indentation), while accepting
/// same-column alignment as a fallback for operator calls. The fallback
/// is needed because RuboCop's `argument_in_method_call` (which requires
/// AST parent traversal) accepts alignment in method-arg and nested-if
/// contexts that we cannot detect from Prism without parent pointers.
///
/// Key fix (2026-04-16): the operator-call fallback above only matches
/// RuboCop's `aligned` style. Under `EnforcedStyle=indented`, RuboCop
/// requires `left_indent + width` for ordinary multiline operator calls,
/// so same-column chains like the bottles examples must be offenses.
///
/// Key fix (2026-04-16): under `EnforcedStyle=indented`, RuboCop still
/// accepts same-column continuations when the assignment RHS starts on the
/// next line via a trailing `\` on the assignment line, and it still applies
/// keyword-condition indentation to nested operator calls under `if`/`while`
/// continuations (for example `if lhs &&\n     rhs >=\n       value`).
/// We mirror both quirks here and include `===`, `=~`, and `!~` in the
/// operator-method set so regex/case-equality conditions are checked too.
///
/// Key fix (2026-04-17): aligned style was still treating every multiline
/// operator call like RuboCop's `argument_in_method_call`, which hid plain
/// same-column chains in method bodies (`"a" +\n"b"`) and bottle-style string
/// concatenations. We now cache a small ancestor-derived context per file and
/// keep the same-column fallback only when the operator call is genuinely
/// nested inside a method argument, so `raise Error,\n"..." +` stays accepted
/// while ordinary method-body chains become offenses again.
///
/// Key fix (2026-04-20): that cached method-argument context was still too
/// permissive in aligned style because operator calls whose left operand started
/// after earlier method arguments (for example `puts a, 1 +\n  2` or
/// `UI.warn "..." +\n  rhs`) fell through the generic `left_col > left_indent`
/// branch and accepted normal `+2` indentation. RuboCop aligns actual method
/// arguments in this situation, so the method-argument branch now wins first
/// and only accepts `right_col == left_col`.
///
/// Key fix (2026-04-25): boolean operators inside method-call arguments use
/// RuboCop's same argument-alignment rule as operator method calls, while
/// assignment context is now derived from AST ancestors instead of only from a
/// same-line `=`. This catches RSpec keyword-argument conditions and case-branch
/// concatenations assigned to a variable, and removes the broad left-column
/// fallback that hid over-indented block-body boolean chains.
pub struct MultilineOperationIndentation;

const OPERATOR_METHODS: &[&[u8]] = &[
    b"+", b"-", b"*", b"/", b"%", b"**", b"==", b"===", b"!=", b"=~", b"!~", b"<", b">", b"<=",
    b">=", b"<=>", b"&", b"|", b"^", b"<<", b">>",
];

#[derive(Clone, Copy, Default)]
struct OperatorContext {
    /// The operator is a method-call argument (no block in between). Used as a
    /// fallback to accept same-column alignment in aligned style.
    method_argument: bool,
    /// AST-detected keyword ancestor where the operator sits inside the
    /// keyword's indented expression (predicate/return value/for collection).
    /// Walks across blocks like RuboCop's `kw_node_with_special_indentation`.
    keyword: Option<KeywordCtx>,
    /// True when an enclosing block body sits between the operator and any
    /// outer assignment ancestor — disqualifies lexical assignment-context
    /// detection (mirrors RuboCop's `disqualified_rhs?` block-body rule and
    /// the UNALIGNED_RHS_TYPES list).
    block_disqualifies_assignment: bool,
    /// Start offset of the AST-detected assignment RHS containing the operator.
    /// Used to mirror RuboCop's `part_of_assignment_rhs` without parent links
    /// during the actual node check.
    assignment_rhs_start: Option<usize>,
}

#[derive(Clone, Copy)]
struct KeywordCtx {
    keyword: &'static str,
    /// True for postfix conditionals (`expr if cond`); RuboCop uses
    /// `width` (not `2 * width`) in that case.
    postfix: bool,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct OpKey {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct CacheKey {
    parse_result_ptr: usize,
    source_ptr: usize,
    source_len: usize,
}

thread_local! {
    static OPERATOR_CONTEXT_CACHE: RefCell<Option<(CacheKey, HashMap<OpKey, OperatorContext>)>> =
        const { RefCell::new(None) };
}

struct OperatorContextVisitor<'pr> {
    ancestors: Vec<ruby_prism::Node<'pr>>,
    contexts: HashMap<OpKey, OperatorContext>,
}

impl<'pr> Visit<'pr> for OperatorContextVisitor<'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {}

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if OPERATOR_METHODS.contains(&node.name().as_slice()) {
            let current = node.as_node();
            self.contexts
                .insert(op_key(&current), build_context(&self.ancestors, &current));
        }

        ruby_prism::visit_call_node(self, node);
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        let current = node.as_node();
        self.contexts
            .insert(op_key(&current), build_context(&self.ancestors, &current));
        ruby_prism::visit_and_node(self, node);
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        let current = node.as_node();
        self.contexts
            .insert(op_key(&current), build_context(&self.ancestors, &current));
        ruby_prism::visit_or_node(self, node);
    }
}

fn op_key(node: &ruby_prism::Node<'_>) -> OpKey {
    let loc = node.location();
    OpKey {
        start: loc.start_offset(),
        end: loc.end_offset(),
    }
}

fn node_within_node(inner: &ruby_prism::Node<'_>, outer: &ruby_prism::Node<'_>) -> bool {
    let inner_loc = inner.location();
    let outer_loc = outer.location();
    inner_loc.start_offset() >= outer_loc.start_offset()
        && inner_loc.end_offset() <= outer_loc.end_offset()
}

/// Returns true if `node` is one of RuboCop's `UNALIGNED_RHS_TYPES`
/// (`if`, `while`, `until`, `for`, `return`, `array`, `kwbegin`).
fn is_unaligned_rhs_type(node: &ruby_prism::Node<'_>) -> bool {
    node.as_if_node().is_some()
        || node.as_unless_node().is_some()
        || node.as_while_node().is_some()
        || node.as_until_node().is_some()
        || node.as_for_node().is_some()
        || node.as_return_node().is_some()
        || node.as_array_node().is_some()
        || node.as_begin_node().is_some()
}

/// Returns true if `node` is a Prism block-like construct (regular block or
/// lambda). Both are RuboCop `block_type?` for `disqualified_rhs?`.
fn is_block_like(node: &ruby_prism::Node<'_>) -> bool {
    node.as_block_node().is_some() || node.as_lambda_node().is_some()
}

/// True if the keyword ancestor's "indented expression" — predicate for
/// `if`/`unless`/`while`/`until`, collection for `for`, value for `return` —
/// contains `current`.
fn within_indented_keyword_expression(
    ancestor: &ruby_prism::Node<'_>,
    current: &ruby_prism::Node<'_>,
) -> Option<KeywordCtx> {
    if let Some(if_node) = ancestor.as_if_node() {
        // Skip ternaries — RuboCop excludes them (`ancestor.if_type? && ancestor.ternary?`).
        // Prism encodes ternaries as IfNode but with `if_keyword_loc()` absent
        // and `then_keyword_loc()` representing the `?`.
        if_node.if_keyword_loc()?;
        let predicate = if_node.predicate();
        if node_within_node(current, &predicate) {
            return Some(KeywordCtx {
                keyword: "if",
                postfix: is_modifier_if(&if_node),
            });
        }
        return None;
    }
    if let Some(unless_node) = ancestor.as_unless_node() {
        let predicate = unless_node.predicate();
        if node_within_node(current, &predicate) {
            return Some(KeywordCtx {
                keyword: "unless",
                postfix: is_modifier_unless(&unless_node),
            });
        }
        return None;
    }
    if let Some(while_node) = ancestor.as_while_node() {
        let predicate = while_node.predicate();
        if node_within_node(current, &predicate) {
            // While modifiers (`expr while cond`) get postfix indentation.
            let postfix =
                while_node.do_keyword_loc().is_none() && while_node.closing_loc().is_none();
            return Some(KeywordCtx {
                keyword: "while",
                postfix,
            });
        }
        return None;
    }
    if let Some(until_node) = ancestor.as_until_node() {
        let predicate = until_node.predicate();
        if node_within_node(current, &predicate) {
            let postfix =
                until_node.do_keyword_loc().is_none() && until_node.closing_loc().is_none();
            return Some(KeywordCtx {
                keyword: "until",
                postfix,
            });
        }
        return None;
    }
    if let Some(for_node) = ancestor.as_for_node() {
        let collection = for_node.collection();
        if node_within_node(current, &collection) {
            return Some(KeywordCtx {
                keyword: "for",
                postfix: false,
            });
        }
        return None;
    }
    if let Some(return_node) = ancestor.as_return_node() {
        if let Some(args) = return_node.arguments() {
            let args_node = args.as_node();
            if node_within_node(current, &args_node) {
                return Some(KeywordCtx {
                    keyword: "return",
                    postfix: false,
                });
            }
        }
        return None;
    }
    None
}

fn build_context(
    ancestors: &[ruby_prism::Node<'_>],
    current: &ruby_prism::Node<'_>,
) -> OperatorContext {
    let mut method_argument = false;
    let mut method_argument_locked = false;
    let mut keyword: Option<KeywordCtx> = None;
    let mut block_disqualifies_assignment = false;
    let mut assignment_rhs_start = None;
    let mut assignment_resolved = false;

    for ancestor in ancestors.iter().rev() {
        // method_argument: stop at first block-like or first matching call.
        if !method_argument_locked {
            if is_block_like(ancestor) {
                method_argument_locked = true;
            } else if let Some(call) = ancestor.as_call_node() {
                if !method_identifier_predicates::is_setter_method(call.name().as_slice()) {
                    let arg_context = call.arguments().and_then(|args| {
                        args.arguments()
                            .iter()
                            .find(|arg| node_within_node(current, arg))
                    });
                    if let Some(arg) = arg_context {
                        method_argument = arg.as_def_node().is_none();
                        method_argument_locked = true;
                    }
                }
                if method_argument {
                    method_argument_locked = true;
                }
            }
        }

        // assignment_rhs disqualification: walk ancestors until we hit an
        // assignment (no disqualification) or a disqualifier (block body or
        // UNALIGNED_RHS_TYPES). RuboCop's `part_of_assignment_rhs` with
        // `disqualified_rhs?` and `valid_rhs?`.
        if !assignment_resolved {
            if is_block_like(ancestor) {
                block_disqualifies_assignment = true;
                assignment_resolved = true;
            } else if is_unaligned_rhs_type(ancestor) {
                // Unaligned RHS types break the assignment search before any
                // outer assignment can be reached.
                block_disqualifies_assignment = true;
                assignment_resolved = true;
            } else if let Some(rhs) = assignment_rhs_node(ancestor) {
                if node_within_node(current, &rhs) {
                    assignment_rhs_start = Some(rhs.location().start_offset());
                    assignment_resolved = true;
                }
            }
        }

        // keyword_special_indentation: closest keyword ancestor where current
        // is inside the indented expression. Walks across blocks.
        if keyword.is_none() {
            keyword = within_indented_keyword_expression(ancestor, current);
        }
    }

    OperatorContext {
        method_argument,
        keyword,
        block_disqualifies_assignment,
        assignment_rhs_start,
    }
}

/// Returns the RHS for assignment-like ancestors. Mirrors RuboCop's
/// `assignment_rhs`: assignment nodes use their value/expression, while setter
/// calls (`obj.foo = bar`) use the last argument.
fn assignment_rhs_node<'pr>(node: &ruby_prism::Node<'pr>) -> Option<ruby_prism::Node<'pr>> {
    if let Some(n) = node.as_local_variable_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_local_variable_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_local_variable_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_local_variable_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_instance_variable_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_instance_variable_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_instance_variable_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_instance_variable_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_class_variable_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_class_variable_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_class_variable_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_class_variable_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_global_variable_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_global_variable_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_global_variable_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_global_variable_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_path_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_path_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_path_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_path_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_index_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_index_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_index_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_call_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_call_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_call_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_multi_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_match_write_node() {
        return Some(n.call().as_node());
    }
    if let Some(call) = node.as_call_node() {
        if method_identifier_predicates::is_setter_method(call.name().as_slice()) {
            return call.arguments().and_then(|args| args.arguments().last());
        }
    }
    None
}

fn operator_context(
    parse_result: &ruby_prism::ParseResult<'_>,
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
) -> OperatorContext {
    let cache_key = CacheKey {
        parse_result_ptr: parse_result as *const _ as usize,
        source_ptr: source.as_bytes().as_ptr() as usize,
        source_len: source.as_bytes().len(),
    };

    OPERATOR_CONTEXT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let needs_rebuild = !matches!(cache.as_ref(), Some((key, _)) if *key == cache_key);

        if needs_rebuild {
            let mut visitor = OperatorContextVisitor {
                ancestors: Vec::new(),
                contexts: HashMap::new(),
            };
            visitor.visit(&parse_result.node());
            *cache = Some((cache_key, visitor.contexts));
        }

        cache
            .as_ref()
            .and_then(|(_, contexts)| contexts.get(&op_key(node)).copied())
            .unwrap_or_default()
    })
}

impl Cop for MultilineOperationIndentation {
    fn name(&self) -> &'static str {
        "Layout/MultilineOperationIndentation"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[AND_NODE, CALL_NODE, OR_NODE]
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
        let style = config.get_str("EnforcedStyle", "aligned");

        // Check CallNode with operator methods (binary operators are parsed as calls)
        if let Some(call_node) = node.as_call_node() {
            let method_name = call_node.name().as_slice();

            if !OPERATOR_METHODS.contains(&method_name) {
                return;
            }

            // Skip if inside a grouped expression or method call arg list parentheses.
            // Matches RuboCop's not_for_this_cop? check for operator method calls.
            if is_inside_parentheses(source, node) {
                return;
            }

            let receiver = match call_node.receiver() {
                Some(r) => r,
                None => return,
            };

            let args_node = match call_node.arguments() {
                Some(a) => a,
                None => return,
            };

            let args: Vec<_> = args_node.arguments().iter().collect();
            if args.is_empty() {
                return;
            }

            let ctx = operator_context(parse_result, source, node);
            let first_arg = &args[0];
            diagnostics
                .extend(self.check_binary_node(source, &receiver, first_arg, config, style, ctx));
            return;
        }

        // Check AndNode
        if let Some(and_node) = node.as_and_node() {
            // Skip if inside a grouped expression (parentheses) or method call
            // arg list parentheses — matches RuboCop's not_for_this_cop? check.
            if is_inside_parentheses(source, node) {
                return;
            }
            let ctx = operator_context(parse_result, source, node);
            diagnostics.extend(self.check_binary_node(
                source,
                &and_node.left(),
                &and_node.right(),
                config,
                style,
                ctx,
            ));
            return;
        }

        // Check OrNode
        if let Some(or_node) = node.as_or_node() {
            // Skip if inside a grouped expression or method call arg list parentheses
            if is_inside_parentheses(source, node) {
                return;
            }
            let ctx = operator_context(parse_result, source, node);
            diagnostics.extend(self.check_binary_node(
                source,
                &or_node.left(),
                &or_node.right(),
                config,
                style,
                ctx,
            ));
        }
    }
}

/// Check if a node is enclosed by parentheses by scanning the source.
/// This matches RuboCop's `not_for_this_cop?` which skips and/or nodes inside
/// grouped expressions `(expr)` or method call arg list parentheses `foo(expr)`.
///
/// We scan backwards from the node's start offset counting unbalanced parens.
/// If we find an unmatched `(` that is also balanced by a `)` after the node's
/// end, the node is inside parentheses.
fn is_inside_parentheses(source: &SourceFile, node: &ruby_prism::Node<'_>) -> bool {
    let bytes = source.as_bytes();
    let node_start = node.location().start_offset();
    let node_end = node.location().end_offset();

    // Scan backwards from node_start to find unmatched '('
    let mut depth = 0i32;
    let mut pos = node_start;
    while pos > 0 {
        pos -= 1;
        match bytes[pos] {
            b')' => depth += 1,
            b'(' => {
                if depth > 0 {
                    depth -= 1;
                } else {
                    // Found an unmatched '(' before the node.
                    // Now verify there's a matching ')' after the node.
                    let mut fwd_depth = 0i32;
                    for &b in &bytes[node_end..] {
                        match b {
                            b'(' => fwd_depth += 1,
                            b')' => {
                                if fwd_depth > 0 {
                                    fwd_depth -= 1;
                                } else {
                                    return true;
                                }
                            }
                            _ => {}
                        }
                    }
                    return false;
                }
            }
            // Don't cross method/class/module boundaries
            b'\n' => {
                // Check if this line starts a method/class def (rough check)
                // We allow scanning through multiple lines within a single expression.
            }
            _ => {}
        }
    }
    false
}

/// Count leading whitespace bytes (spaces and tabs) on a line.
/// Unlike `shared::util::indentation_of` which only counts spaces, this
/// also counts tabs — needed for finding the first non-whitespace token
/// in keyword context detection.
fn leading_whitespace_len_with_tabs(line: &[u8]) -> usize {
    line.iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count()
}

#[derive(Clone, Copy)]
struct KeywordContext {
    keyword: &'static str,
    special_indentation: bool,
}

#[derive(Clone, Copy)]
struct AssignmentContext {
    rhs_begins_line: bool,
}

fn last_significant_index(line_bytes: &[u8]) -> Option<usize> {
    line_bytes
        .iter()
        .rposition(|&b| b != b' ' && b != b'\t' && b != b'\r' && b != b'\n')
}

fn is_assignment_operator(bytes: &[u8], idx: usize) -> bool {
    if bytes.get(idx) != Some(&b'=') {
        return false;
    }
    if bytes.get(idx + 1) == Some(&b'=') {
        return false;
    }
    !matches!(
        idx.checked_sub(1).and_then(|i| bytes.get(i)),
        Some(b'=' | b'!' | b'<' | b'>')
    )
}

fn has_assignment_before_col(line_bytes: &[u8], col: usize) -> bool {
    let end = col.min(line_bytes.len());
    (0..end)
        .rev()
        .find(|&idx| line_bytes[idx] == b'=')
        .is_some_and(|idx| is_assignment_operator(line_bytes, idx))
}

fn line_ends_with_assignment_operator(line_bytes: &[u8]) -> bool {
    let mut idx = match last_significant_index(line_bytes) {
        Some(idx) => idx,
        None => return false,
    };

    if line_bytes[idx] == b'\\' {
        idx = match last_significant_index(&line_bytes[..idx]) {
            Some(idx) => idx,
            None => return false,
        };
    }

    is_assignment_operator(line_bytes, idx)
}

fn line_ends_with_logical_operator(line_bytes: &[u8]) -> bool {
    let Some(idx) = last_significant_index(line_bytes) else {
        return false;
    };
    let trimmed = &line_bytes[..=idx];
    trimmed.ends_with(b"&&")
        || trimmed.ends_with(b"||")
        || trimmed.ends_with(b" and")
        || trimmed.ends_with(b" or")
}

fn modifier_keyword(before_expr: &[u8]) -> Option<&'static str> {
    if before_expr.windows(8).any(|w| w == b" unless ")
        || before_expr.windows(8).any(|w| w == b" unless(")
    {
        Some("unless")
    } else if before_expr.windows(7).any(|w| w == b" while ")
        || before_expr.windows(7).any(|w| w == b" while(")
    {
        Some("while")
    } else if before_expr.windows(7).any(|w| w == b" until ")
        || before_expr.windows(7).any(|w| w == b" until(")
    {
        Some("until")
    } else if before_expr.windows(4).any(|w| w == b" if ")
        || before_expr.windows(4).any(|w| w == b" if(")
    {
        Some("if")
    } else {
        None
    }
}

fn keyword_context_on_line(
    source: &SourceFile,
    line: usize,
    expr_col: usize,
) -> Option<KeywordContext> {
    fn extract(line_bytes: &[u8], expr_col: usize) -> Option<KeywordContext> {
        let start = leading_whitespace_len_with_tabs(line_bytes);
        let end = expr_col.min(line_bytes.len());
        let before_expr = &line_bytes[start..end];

        if before_expr.starts_with(b"elsif ") {
            return Some(KeywordContext {
                keyword: "elsif",
                special_indentation: true,
            });
        }
        if before_expr.starts_with(b"if ") || before_expr.starts_with(b"if(") {
            return Some(KeywordContext {
                keyword: "if",
                special_indentation: true,
            });
        }
        if before_expr.starts_with(b"unless ") || before_expr.starts_with(b"unless(") {
            return Some(KeywordContext {
                keyword: "unless",
                special_indentation: true,
            });
        }
        if before_expr.starts_with(b"while ") || before_expr.starts_with(b"while(") {
            return Some(KeywordContext {
                keyword: "while",
                special_indentation: true,
            });
        }
        if before_expr.starts_with(b"until ") || before_expr.starts_with(b"until(") {
            return Some(KeywordContext {
                keyword: "until",
                special_indentation: true,
            });
        }
        if before_expr.starts_with(b"for ") {
            return Some(KeywordContext {
                keyword: "for",
                special_indentation: true,
            });
        }
        if before_expr.starts_with(b"return ") {
            if let Some(keyword) = modifier_keyword(before_expr) {
                return Some(KeywordContext {
                    keyword,
                    special_indentation: false,
                });
            }
            return Some(KeywordContext {
                keyword: "return",
                special_indentation: true,
            });
        }
        modifier_keyword(before_expr).map(|keyword| KeywordContext {
            keyword,
            special_indentation: false,
        })
    }

    let line_bytes = source.lines().nth(line - 1).unwrap_or(b"");
    if let Some(ctx) = extract(line_bytes, expr_col) {
        return Some(ctx);
    }

    if line > 1 {
        let prev_line = source.lines().nth(line - 2).unwrap_or(b"");
        if last_significant_index(prev_line).is_some_and(|idx| prev_line[idx] == b'\\') {
            return extract(prev_line, prev_line.len());
        }

        let line_indent = leading_whitespace_len_with_tabs(line_bytes);
        let prev_indent = leading_whitespace_len_with_tabs(prev_line);
        if prev_indent < line_indent && line_ends_with_logical_operator(prev_line) {
            if let Some(ctx) = extract(prev_line, prev_line.len()) {
                return Some(ctx);
            }
        }
    }
    None
}

fn assignment_context(
    source: &SourceFile,
    left_line: usize,
    left_col: usize,
) -> Option<AssignmentContext> {
    let left_line_bytes = source.lines().nth(left_line - 1).unwrap_or(b"");
    if has_assignment_before_col(left_line_bytes, left_col) {
        return Some(AssignmentContext {
            rhs_begins_line: false,
        });
    }

    if left_line > 1 {
        let prev_line = source.lines().nth(left_line - 2).unwrap_or(b"");
        // RuboCop only treats the expression as "assignment-aligned" when the
        // operation itself begins the continued RHS line. Nested keyword/body
        // expressions on that line (for example `value = \n  if a ||\n     b`)
        // still use their own indentation rules.
        if line_ends_with_assignment_operator(prev_line)
            && left_col == leading_whitespace_len_with_tabs(left_line_bytes)
        {
            return Some(AssignmentContext {
                rhs_begins_line: true,
            });
        }
    }
    None
}

fn operation_description(
    keyword_context: Option<KeywordContext>,
    assignment_context: Option<AssignmentContext>,
) -> String {
    if let Some(ctx) = keyword_context {
        let kind = if ctx.keyword == "for" {
            "collection"
        } else {
            "condition"
        };
        let article = if ctx.keyword.starts_with('i') || ctx.keyword.starts_with('u') {
            "an"
        } else {
            "a"
        };
        format!("a {kind} in {article} `{}` statement", ctx.keyword)
    } else if assignment_context.is_some() {
        "an expression in an assignment".to_string()
    } else {
        "an expression".to_string()
    }
}

impl MultilineOperationIndentation {
    fn check_binary_node(
        &self,
        source: &SourceFile,
        left: &ruby_prism::Node<'_>,
        right: &ruby_prism::Node<'_>,
        config: &CopConfig,
        style: &str,
        ctx: OperatorContext,
    ) -> Vec<Diagnostic> {
        let (left_line, left_col) = source.offset_to_line_col(left.location().start_offset());
        let (left_end_line, _) = source.offset_to_line_col(left.location().end_offset());
        let (right_line, right_col) = source.offset_to_line_col(right.location().start_offset());

        // Use end of left operand for same-line check. For chained ||/&&
        // like `a || b || c`, the outer Or has left=Or(a,b) spanning lines
        // but `c` may be on the same line as `b` (the end of the left subtree).
        if right_line == left_end_line {
            return Vec::new();
        }

        // RuboCop's `begins_its_line?` — only check if the right operand is
        // the first non-whitespace on its line. When the operator is leading
        // (e.g., `expr \n  && other_expr`), the right operand is NOT the first
        // token on the line and RuboCop skips the check.
        if !begins_its_line(source, right.location().start_offset()) {
            return Vec::new();
        }

        let width = config.get_usize("IndentationWidth", 2);

        // For chained boolean expressions like And(And(a,b), c), the left
        // operand's start_offset points to `a`'s position (the root of the
        // chain). This gives us the correct base indentation.
        let left_line_bytes = source.lines().nth(left_line - 1).unwrap_or(b"");
        let left_indent = indentation_of(left_line_bytes);
        let lexical_keyword = keyword_context_on_line(source, left_line, left_col);
        // AST-derived keyword wins when it disagrees: the ancestor walk catches
        // multi-line keyword conditions (`return [] unless\n  cond1 && cond2`)
        // that lexical scanning of the same line would miss.
        let keyword_context = ctx.keyword.map_or(lexical_keyword, |kw| {
            Some(KeywordContext {
                keyword: kw.keyword,
                special_indentation: !kw.postfix,
            })
        });
        let lexical_assignment = assignment_context(source, left_line, left_col);
        let ast_assignment = ctx.assignment_rhs_start.map(|start| AssignmentContext {
            rhs_begins_line: begins_its_line(source, start),
        });
        // Block-body or UNALIGNED_RHS_TYPES ancestors disqualify the lexical `=`
        // detection from being treated as the operator's own assignment context
        // — mirrors RuboCop's `disqualified_rhs?` (`block_type? && part_of_block_body?`
        // and the UNALIGNED_RHS_TYPES list).
        let assignment_context = if ctx.block_disqualifies_assignment {
            None
        } else {
            ast_assignment.or(lexical_assignment)
        };
        let should_align = assignment_context.is_some_and(|c| c.rhs_begins_line)
            || (style == "aligned" && (keyword_context.is_some() || assignment_context.is_some()));
        let align_only = should_align || (ctx.method_argument && style == "aligned");
        let expected_indent = left_indent
            + if keyword_context.is_some_and(|c| c.special_indentation) {
                2 * width
            } else {
                width
            };

        let is_ok = if align_only {
            right_col == left_col
        } else {
            right_col == expected_indent
        };

        if !is_ok {
            let message = if align_only {
                format!(
                    "Align the operands of {} spanning multiple lines.",
                    operation_description(keyword_context, assignment_context)
                )
            } else {
                let used = right_col.saturating_sub(left_indent);
                format!(
                    "Use {} (not {used}) spaces for indenting {} spanning multiple lines.",
                    expected_indent.saturating_sub(left_indent),
                    operation_description(keyword_context, assignment_context)
                )
            };
            return vec![self.diagnostic(source, right_line, right_col, message)];
        }

        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        MultilineOperationIndentation,
        "cops/layout/multiline_operation_indentation"
    );
    crate::cop_variant_fixture_tests!(
        MultilineOperationIndentation,
        "cops/layout/multiline_operation_indentation",
        indented
    );

    #[test]
    fn single_line_operation_ignored() {
        let source = b"x = 1 + 2\n";
        let diags = run_cop_full(&MultilineOperationIndentation, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn or_in_def_body_no_offense() {
        let src = b"def valid?(user)\n  user.foo ||\n    user.bar\nend\n";
        let diags = run_cop_full(&MultilineOperationIndentation, src);
        assert!(
            diags.is_empty(),
            "correctly indented || continuation should not flag, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn or_in_def_body_with_rescue_no_offense() {
        let src = b"  def valid_otp_attempt?(user)\n    user.validate_and_consume_otp!(user_params[:otp_attempt]) ||\n      user.invalidate_otp_backup_code!(user_params[:otp_attempt])\n  rescue OpenSSL::Cipher::CipherError\n    false\n  end\n";
        let diags = run_cop_full(&MultilineOperationIndentation, src);
        assert!(
            diags.is_empty(),
            "correctly indented || with rescue should not flag, got: {:?}",
            diags
                .iter()
                .map(|d| format!(
                    "line {} col {} {}",
                    d.location.line, d.location.column, d.message
                ))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn nested_and_or_deep_indent_no_offense() {
        let src = b"        def implicit_block?(node)\n          return false unless node.arguments.any?\n\n          node.last_argument.block_pass_type? ||\n            (node.last_argument.sym_type? &&\n            methods_accepting_symbol.include?(node.method_name.to_s))\n        end\n";
        let diags = run_cop_full(&MultilineOperationIndentation, src);
        assert!(
            diags.is_empty(),
            "nested && inside || with aligned continuation should not flag, got: {:?}",
            diags
                .iter()
                .map(|d| format!(
                    "line {} col {} {}",
                    d.location.line, d.location.column, d.message
                ))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn aligned_style() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("aligned".into()),
            )]),
            ..CopConfig::default()
        };
        // Aligned with left operand in keyword condition (should_align = true)
        let src = b"if a &&\n   b\n  c\nend\n";
        let diags = run_cop_full_with_config(&MultilineOperationIndentation, src, config.clone());
        assert!(
            diags.is_empty(),
            "aligned style in keyword condition should accept operand-aligned continuation, got: {:?}",
            diags
                .iter()
                .map(|d| format!("L{}:C{} {}", d.location.line, d.location.column, &d.message))
                .collect::<Vec<_>>()
        );

        // In "aligned" style, ordinary statements still use indentation.
        let src2 = b"a &&\n  b\n";
        let diags2 = run_cop_full_with_config(&MultilineOperationIndentation, src2, config.clone());
        assert!(
            diags2.is_empty(),
            "aligned style should accept indented continuation in non-condition contexts"
        );

        // Assignment RHS uses aligned operands in aligned style.
        let src3 = b"x = a &&\n    b\n";
        let diags3 = run_cop_full_with_config(&MultilineOperationIndentation, src3, config);
        assert!(
            diags3.is_empty(),
            "aligned style should accept operand-aligned continuation in assignments"
        );

        let src4 = b"x = a &&\n  b\n";
        let diags4 = run_cop_full_with_config(
            &MultilineOperationIndentation,
            src4,
            CopConfig {
                options: HashMap::from([(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("aligned".into()),
                )]),
                ..CopConfig::default()
            },
        );
        assert_eq!(
            diags4.len(),
            1,
            "aligned style should flag indented assignment continuations"
        );
    }

    #[test]
    fn aligned_style_accepts_modifier_keyword_alignment() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let src = b"def f\n  return if receiver.nil? &&\n            args.empty?\nend\n";
        let diags = run_cop_full_with_config(
            &MultilineOperationIndentation,
            src,
            CopConfig {
                options: HashMap::from([(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("aligned".into()),
                )]),
                ..CopConfig::default()
            },
        );
        assert!(
            diags.is_empty(),
            "modifier keyword conditions should align like RuboCop, got: {:?}",
            diags
                .iter()
                .map(|d| format!("L{}:C{} {}", d.location.line, d.location.column, &d.message))
                .collect::<Vec<_>>()
        );
    }
}
