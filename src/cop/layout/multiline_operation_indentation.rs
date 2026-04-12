use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ruby_prism::Visit;

use crate::cop::shared::method_identifier_predicates;
use crate::cop::shared::node_type::{AND_NODE, CALL_NODE, OR_NODE};
use crate::cop::shared::util::{begins_its_line, indentation_of};
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
/// Key fix (2026-04-12): replaced the unconditional same-column fallback for
/// operator calls with real ancestor-based method-argument detection. Plain
/// chained `+` calls in method bodies like `"a" +\n"b"` are offenses, but
/// the same shape remains accepted when it is actually a method argument
/// (for example `raise Error,\n"a" +\n"b"` or `it "a" +\n"b" do ... end`).
/// Keep the old permissive fallback for `EnforcedStyle=indented`; that variant
/// still has separate corpus divergence and tightening it here caused large
/// regressions unrelated to the aligned-style FN fix.
pub struct MultilineOperationIndentation;

const OPERATOR_METHODS: &[&[u8]] = &[
    b"+", b"-", b"*", b"/", b"%", b"**", b"==", b"!=", b"<", b">", b"<=", b">=", b"<=>", b"&",
    b"|", b"^", b"<<", b">>",
];

#[derive(Clone, Copy, Default)]
struct CallContext {
    method_argument: bool,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct CallKey {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct CacheKey {
    parse_result_ptr: usize,
    source_path_hash: u64,
    source_len: usize,
}

thread_local! {
    static CALL_CONTEXT_CACHE: RefCell<Option<(CacheKey, HashMap<CallKey, CallContext>)>> =
        const { RefCell::new(None) };
}

struct CallContextVisitor<'pr> {
    ancestors: Vec<ruby_prism::Node<'pr>>,
    contexts: HashMap<CallKey, CallContext>,
}

impl<'pr> Visit<'pr> for CallContextVisitor<'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {}

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.contexts.insert(
            call_key(node),
            CallContext {
                method_argument: operator_call_argument_context(&self.ancestors, node),
            },
        );

        ruby_prism::visit_call_node(self, node);
    }
}

fn call_key(call: &ruby_prism::CallNode<'_>) -> CallKey {
    let loc = call.location();
    CallKey {
        start: loc.start_offset(),
        end: loc.end_offset(),
    }
}

fn location_within_offsets(
    inner_start: usize,
    inner_end: usize,
    outer: ruby_prism::Location<'_>,
) -> bool {
    inner_start >= outer.start_offset() && inner_end <= outer.end_offset()
}

fn operator_call_argument_context(
    ancestors: &[ruby_prism::Node<'_>],
    node: &ruby_prism::CallNode<'_>,
) -> bool {
    let current = node.location();
    let current_start = current.start_offset();
    let current_end = current.end_offset();
    let len = ancestors.len().saturating_sub(1);

    for ancestor in ancestors[..len].iter().rev() {
        if ancestor.as_block_node().is_some() {
            return false;
        }

        let Some(call) = ancestor.as_call_node() else {
            continue;
        };

        if method_identifier_predicates::is_setter_method(call.name().as_slice()) {
            continue;
        }

        if call.arguments().is_some_and(|args| {
            args.arguments()
                .iter()
                .any(|arg| location_within_offsets(current_start, current_end, arg.location()))
        }) {
            return true;
        }
    }

    false
}

fn call_context(
    parse_result: &ruby_prism::ParseResult<'_>,
    source: &SourceFile,
    call: &ruby_prism::CallNode<'_>,
) -> CallContext {
    let mut path_hasher = DefaultHasher::new();
    source.path_str().hash(&mut path_hasher);
    let cache_key = CacheKey {
        parse_result_ptr: parse_result as *const _ as usize,
        source_path_hash: path_hasher.finish(),
        source_len: source.as_bytes().len(),
    };

    CALL_CONTEXT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let needs_rebuild = !matches!(cache.as_ref(), Some((key, _)) if *key == cache_key);

        if needs_rebuild {
            let mut visitor = CallContextVisitor {
                ancestors: Vec::new(),
                contexts: HashMap::new(),
            };
            visitor.visit(&parse_result.node());
            *cache = Some((cache_key, visitor.contexts));
        }

        cache
            .as_ref()
            .and_then(|(_, contexts)| contexts.get(&call_key(call)).copied())
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

            let first_arg = &args[0];
            let context = call_context(parse_result, source, &call_node);
            diagnostics.extend(self.check_binary_node(
                source,
                &receiver,
                first_arg,
                config,
                style,
                context.method_argument,
                true,
            ));
            return;
        }

        // Check AndNode
        if let Some(and_node) = node.as_and_node() {
            // Skip if inside a grouped expression (parentheses) or method call
            // arg list parentheses — matches RuboCop's not_for_this_cop? check.
            if is_inside_parentheses(source, node) {
                return;
            }
            diagnostics.extend(self.check_binary_node(
                source,
                &and_node.left(),
                &and_node.right(),
                config,
                style,
                false,
                false,
            ));
            return;
        }

        // Check OrNode
        if let Some(or_node) = node.as_or_node() {
            // Skip if inside a grouped expression or method call arg list parentheses
            if is_inside_parentheses(source, node) {
                return;
            }
            diagnostics.extend(self.check_binary_node(
                source,
                &or_node.left(),
                &or_node.right(),
                config,
                style,
                false,
                false,
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
    last_significant_index(line_bytes).is_some_and(|idx| is_assignment_operator(line_bytes, idx))
}

fn line_ends_with_backslash(line_bytes: &[u8]) -> bool {
    last_significant_index(line_bytes).is_some_and(|idx| line_bytes[idx] == b'\\')
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
        if line_ends_with_assignment_operator(prev_line) {
            return Some(AssignmentContext {
                rhs_begins_line: true,
            });
        }

        if line_ends_with_backslash(prev_line) {
            let mut continued_line = left_line - 1;
            loop {
                let line_bytes = source.lines().nth(continued_line - 1).unwrap_or(b"");
                let end =
                    last_significant_index(line_bytes).map_or(line_bytes.len(), |idx| idx + 1);
                if has_assignment_before_col(line_bytes, end) {
                    return Some(AssignmentContext {
                        rhs_begins_line: true,
                    });
                }

                if continued_line <= 1 {
                    break;
                }

                let above = source.lines().nth(continued_line - 2).unwrap_or(b"");
                if !line_ends_with_backslash(above) {
                    break;
                }
                continued_line -= 1;
            }
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
        method_argument_context: bool,
        operator_call: bool,
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
        let keyword_context = keyword_context_on_line(source, left_line, left_col);
        let assignment_context = assignment_context(source, left_line, left_col);
        let should_align = assignment_context.is_some_and(|ctx| ctx.rhs_begins_line)
            || (style == "aligned"
                && (keyword_context.is_some()
                    || assignment_context.is_some()
                    || method_argument_context));
        let expected_indent = left_indent
            + if keyword_context.is_some_and(|ctx| ctx.special_indentation) {
                2 * width
            } else {
                width
            };

        let is_ok = if should_align {
            right_col == left_col
        } else if style == "aligned" && left_col > left_indent {
            // In aligned style without a keyword/assignment context (e.g. boolean
            // chains in hash values or method args), accept operand-aligned
            // continuations when the left operand is offset from the base indent
            // (genuine alignment, not just same-indent-level chains).
            right_col == expected_indent || right_col == left_col
        } else if operator_call && style == "indented" {
            right_col == expected_indent || right_col == left_col
        } else {
            right_col == expected_indent
        };

        if !is_ok {
            let message = if should_align {
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
