use ruby_prism::Visit;

use crate::cop::shared::method_identifier_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Layout/FirstArgumentIndentation checks the indentation of the first argument
/// in a method call (both parenthesized and non-parenthesized).
///
/// ## Investigation (2026-03-14)
///
/// **FN root cause (104 FNs, 76 from `whenever`):** The cop previously only checked
/// parenthesized calls (requiring `opening_loc == "("`). RuboCop's `on_send` checks
/// ALL calls with arguments, including non-parenthesized ones like:
///   `Whenever.cron \`
///   `<<-file`
/// Fixed by removing the parenthesis requirement and checking all calls with arguments.
///
/// **FP root causes (6 FPs):**
/// 1. `end.(params[:q], params[:t])` — lambda/proc `.call()` on block end. The
///    `call_start_offset` points to the `begin` keyword (far above), making the
///    call appear multi-line. Fixed by using `call_operator_loc` or `opening_loc`
///    to determine the call line when `message_loc` is absent.
/// 2. String interpolation `#{builder.attachment(\n  :image,` — calls inside
///    heredoc interpolation have meaningless indentation context. Fixed by tracking
///    interpolation depth and skipping calls inside interpolation.
/// 3. Tab-indented code — the tab indentation case is handled by the existing
///    indentation calculation but misfires when tabs mix with spaces. This is a
///    minor edge case that would need tab-width-aware indentation to fix properly.
///
/// ## Investigation (2026-03-15)
///
/// **FP fix (1 FP):** Tab-indented code (mixed tabs/spaces) caused false positives
/// because `indentation_of` counts only spaces while `offset_to_line_col` counts
/// tabs as 1 character, creating mismatches. Fixed by skipping the check entirely
/// when either the argument line or the previous code line has tab indentation.
///
/// **FN fix (2 FN):** `super()` calls were not handled because Prism uses
/// `SuperNode` (not `CallNode`) for explicit `super(args)`. Added
/// `visit_super_node` to the visitor.
///
/// **Remaining FN (11):** 10 FNs from calls inside string interpolation in
/// heredocs (puppetlabs, antiwork, autolab). The `in_interpolation` skip was
/// added to fix FPs, and removing it would reintroduce those FPs. Fixing this
/// would require distinguishing "interpolation with meaningful indentation
/// context" from "interpolation where indentation is relative to the heredoc
/// body." 1 FN from tab-indented code where RuboCop does flag the offense
/// (charlotte-ruby) — would require tab-width-aware column counting.
///
/// ## Investigation (2026-03-18)
///
/// **FP fix (5 FPs):** All 5 FPs were inner calls inside `super()` (consuldemocracy,
/// ManageIQ, gimite, phusion). RuboCop's `eligible_method_call?` only matches
/// `:send` nodes, NOT `:super` nodes. So `super()` should not be an eligible
/// parent for the `special_for_inner_method_call_in_parentheses` check. Fixed by
/// setting `is_eligible: false` in the ParentCallInfo pushed by visit_super_node.
///
/// **FN fix (103 FNs):** Tab-indented code (phlex 93, loomio 4, charlotte-ruby 1,
/// digininja 1, moneta 1, pact 1, redcar 1, peritor 1) was being skipped entirely
/// by tab-detection guards. RuboCop's `previous_code_line =~ /\S/` counts both tabs
/// and spaces as 1 character. Fixed by replacing `indentation_of` (spaces only)
/// with `leading_whitespace_count` (tabs + spaces) and removing the tab-skip guards.
///
/// **FN fix (2026-03-23, 11 FNs):** Removed the blanket `in_interpolation`
/// skip — RuboCop checks calls inside interpolation normally. The original FP
/// that motivated the skip was actually caused by `previous_code_line_indent`
/// treating `#{...}` lines as comments (because `#` is the first non-whitespace
/// char). Fixed by not treating `#` as a comment when followed by `{`.
///
/// ## Investigation (2026-03-29)
///
/// **FN fix (2 FNs):** Dotted operator sends like `Sequel.|(` were being
/// skipped because `is_bare_operator` only looked at the method name. RuboCop
/// only skips true bare operators (`foo + bar`) and still checks dotted sends
/// (`self.+(`, `Sequel.|(`), while safe-navigation operator sends remain
/// skipped because `dot?` is false for `&.`. Fixed by matching RuboCop's
/// `operator_method? && !dot?` behavior and by emitting the quoted base-range
/// message for single-line special inner calls like ``Sequel.|(``.
///
/// ## Investigation (2026-03-31)
///
/// **FP fix (1 FP):** `include(inner_call(...) { ... })` was incorrectly
/// treated as a `special_for_inner_method_call_in_parentheses` case. In
/// RuboCop's parser AST, a send with an attached block is wrapped in a `block`
/// node, so the inner send's parent is not the outer `send` and the special
/// inner-call indentation rule does not apply. Prism keeps the block on the
/// `CallNode`, so the previous stack-based parent check misclassified this
/// shape. Fixed by excluding calls with attached blocks from the special
/// inner-call path while keeping plain parenthesized inner calls unchanged.
///
/// **FN fix (3 FNs):** `expect(Foo.bar(..., &blk))` was still being skipped as
/// a special inner call because Prism also stores block-pass arguments in
/// `call.block()`, but RuboCop only excludes real attached blocks from this
/// path. Fixed by treating only `BlockNode` values as attached blocks and
/// keeping `BlockArgumentNode` (`&blk`) eligible for special inner-call
/// indentation and the quoted base-range message.
pub struct FirstArgumentIndentation;

impl Cop for FirstArgumentIndentation {
    fn name(&self) -> &'static str {
        "Layout/FirstArgumentIndentation"
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
        let style = config.get_str(
            "EnforcedStyle",
            "special_for_inner_method_call_in_parentheses",
        );
        let width = config.get_usize("IndentationWidth", 2);
        let mut visitor = FirstArgVisitor {
            cop: self,
            source,
            style,
            width,
            diagnostics: Vec::new(),
            // Stack of parent call info: (is_parenthesized, call_start_offset)
            parent_call_stack: Vec::new(),
            splat_offsets: Vec::new(),
            splat_expr_depth: 0,
            call_depth: 0,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct FirstArgVisitor<'a> {
    cop: &'a FirstArgumentIndentation,
    source: &'a SourceFile,
    style: &'a str,
    width: usize,
    diagnostics: Vec<Diagnostic>,
    /// Stack of parent call info: (is_parenthesized, call_node_start_col)
    /// call_node_start_col is the column of the start of the entire call expression
    /// (including receiver), matching RuboCop's node.source_range.begin_pos
    parent_call_stack: Vec<ParentCallInfo>,
    /// Stack of splat node start offsets. When non-empty, we're inside a splat.
    /// Used by consistent_relative_to_receiver to match RuboCop's behavior.
    splat_offsets: Vec<usize>,
    /// Depth of CallNode nesting inside splat expressions.
    /// Incremented when we manually visit a CallNode as the expression of a splat.
    /// Decremented after visiting that CallNode's children.
    /// This is different from call_depth because call_depth tracks ALL CallNode visits,
    /// while this tracks only splat expression visits.
    splat_expr_depth: usize,
    /// Depth of CallNode nesting. Incremented when entering a CallNode and
    /// decremented when exiting. When > 0 and splat_offsets is non-empty,
    /// we're inside a call that is the direct child of a splat.
    call_depth: usize,
}

struct ParentCallInfo {
    /// Whether the parent call has parenthesized arguments
    is_parenthesized: bool,
    /// The start offsets of each argument in the parent call, so we can check
    /// if the current call node is one of the parent's arguments
    arg_start_offsets: Vec<usize>,
    /// Not a setter or bare operator
    is_eligible: bool,
}

struct CallMetadata<'a> {
    name: &'a str,
    has_attached_block: bool,
}

impl FirstArgVisitor<'_> {
    fn check_call(
        &mut self,
        call_start_offset: usize,
        message_loc: Option<ruby_prism::Location<'_>>,
        opening_loc: Option<ruby_prism::Location<'_>>,
        call_operator_loc: Option<ruby_prism::Location<'_>>,
        arguments: Option<ruby_prism::ArgumentsNode<'_>>,
        metadata: CallMetadata<'_>,
    ) {
        // Must have arguments (parenthesized or not)
        let args_node = match arguments {
            Some(a) => a,
            None => return,
        };
        let has_regular_dot = call_operator_loc
            .as_ref()
            .is_some_and(|loc| loc.as_slice() == b".");

        let args: Vec<_> = args_node.arguments().iter().collect();
        if args.is_empty() {
            return;
        }

        let first_arg = &args[0];

        // Use message_loc (method name) for determining the call line.
        // This handles chained calls where call_start_offset would be on
        // a different line (the receiver's line).
        // For `end.(args)` calls, message_loc is None but call_start_offset
        // points to the beginning of the block (far above). Use call_operator_loc
        // or opening_loc as fallback.
        let call_line_offset = message_loc
            .map(|loc| loc.start_offset())
            .or_else(|| call_operator_loc.map(|loc| loc.start_offset()))
            .or_else(|| opening_loc.map(|loc| loc.start_offset()))
            .unwrap_or(call_start_offset);
        let (call_line, _) = self.source.offset_to_line_col(call_line_offset);

        let first_arg_loc = first_arg.location();
        let (arg_line, arg_col) = self.source.offset_to_line_col(first_arg_loc.start_offset());

        // Skip if first arg is on same line as method call
        if arg_line == call_line {
            return;
        }

        // Skip bare operators (like `a + b`) and setter methods (like `self.x = 1`)
        if is_bare_operator(metadata.name, has_regular_dot) || is_setter_method(metadata.name) {
            return;
        }

        let expected = self.compute_expected_indent(
            call_start_offset,
            first_arg_loc.start_offset(),
            arg_line,
            metadata.has_attached_block,
        );

        if arg_col != expected {
            self.diagnostics.push(self.cop.diagnostic(
                self.source,
                arg_line,
                arg_col,
                self.message(
                    call_start_offset,
                    first_arg_loc.start_offset(),
                    metadata.has_attached_block,
                ),
            ));
        }
    }

    fn compute_expected_indent(
        &self,
        call_start_offset: usize,
        first_arg_start_offset: usize,
        arg_line: usize,
        has_attached_block: bool,
    ) -> usize {
        if self.style == "consistent" {
            // Always use previous code line indent + width
            return previous_code_line_indent(self.source, arg_line) + self.width;
        }

        if self.style == "consistent_relative_to_receiver" {
            // Use the column of the call node start (includes receiver) + width.
            // But if inside a splat context (e.g., *Dir.glob(...)), RuboCop uses
            // the splat's column as the base, not the call's column.
            // splat_expr_depth > 0 means we're the direct expression of a splat,
            // so we use the splat's offset for expected indentation.
            let base_offset = if self.splat_expr_depth > 0
                && self.call_depth == 2
                && !self.splat_offsets.is_empty()
            {
                // Direct expression of a splat - use the splat's offset
                self.splat_offsets
                    .last()
                    .copied()
                    .unwrap_or(call_start_offset)
            } else {
                call_start_offset
            };
            let (_, base_col) = self.source.offset_to_line_col(base_offset);
            return base_col + self.width;
        }

        // special_for_inner_method_call or special_for_inner_method_call_in_parentheses
        if self.is_special_inner_call(call_start_offset, has_attached_block) {
            // Check if base_range (from call start to first arg) spans multiple lines
            let (call_start_line, call_start_col) =
                self.source.offset_to_line_col(call_start_offset);
            let (arg_start_line, _) = self.source.offset_to_line_col(first_arg_start_offset);

            // Determine if the range from call start to arg start is "single line"
            // after stripping whitespace (matching RuboCop's column_of behavior)
            if is_single_line_base_range(
                self.source,
                call_start_offset,
                first_arg_start_offset,
                call_start_line,
                arg_start_line,
            ) {
                // Single-line: use the column of the call expression start
                call_start_col + self.width
            } else {
                // Multi-line: use previous code line indent
                previous_code_line_indent(self.source, arg_line) + self.width
            }
        } else {
            // Not a special inner call: use previous code line indent + width
            previous_code_line_indent(self.source, arg_line) + self.width
        }
    }

    /// Check if the current call is a "special inner call" — meaning it is an
    /// argument of an outer method call. For `special_for_inner_method_call_in_parentheses`,
    /// the outer call must be parenthesized.
    fn is_special_inner_call(&self, call_start_offset: usize, has_attached_block: bool) -> bool {
        if has_attached_block {
            return false;
        }

        if let Some(parent) = self.parent_call_stack.last() {
            if !parent.is_eligible {
                return false;
            }

            if self.style == "special_for_inner_method_call_in_parentheses"
                && !parent.is_parenthesized
            {
                return false;
            }

            // The call must be an argument of the parent (not just any descendant).
            // We check if call_start_offset matches any of the parent's argument
            // start offsets or is contained within one of them.
            // Actually, RuboCop checks: node.source_range.begin_pos > parent.source_range.begin_pos
            // which means the inner call starts inside the parent call (not being the
            // first part of a chained call).
            // Since we're inside the parent's argument visitor, we know we're a descendant.
            // We just need to verify the call starts after the parent call start
            // (which is always true for arguments).
            // But we also need to make sure the current call IS a direct argument,
            // not just a deeply nested expression. For simplicity, we check if
            // call_start_offset appears in the parent's arg_start_offsets.
            // Actually, RuboCop just checks that the node is a direct child argument
            // of the parent. It walks up one level via node.parent. We can simulate
            // this by checking if the call_start_offset matches one of the parent's
            // argument start offsets.
            parent.arg_start_offsets.contains(&call_start_offset)
        } else {
            false
        }
    }

    fn uses_base_range_message(&self, call_start_offset: usize, has_attached_block: bool) -> bool {
        match self.style {
            "consistent" => false,
            "consistent_relative_to_receiver" => true,
            _ => self.is_special_inner_call(call_start_offset, has_attached_block),
        }
    }

    fn message(
        &self,
        call_start_offset: usize,
        first_arg_start_offset: usize,
        has_attached_block: bool,
    ) -> String {
        let base_text = self
            .source
            .try_byte_slice(call_start_offset, first_arg_start_offset)
            .unwrap_or("")
            .trim();
        let (call_start_line, _) = self.source.offset_to_line_col(call_start_offset);
        let (arg_start_line, _) = self.source.offset_to_line_col(first_arg_start_offset);

        let base = if self.uses_base_range_message(call_start_offset, has_attached_block)
            && !base_text.contains('\n')
            && is_single_line_base_range(
                self.source,
                call_start_offset,
                first_arg_start_offset,
                call_start_line,
                arg_start_line,
            ) {
            format!("`{base_text}`")
        } else if base_text
            .lines()
            .last()
            .is_some_and(is_comment_line_for_message)
        {
            "the start of the previous line (not counting the comment)".to_string()
        } else {
            "the start of the previous line".to_string()
        };

        format!("Indent the first argument one step more than {base}.")
    }
}

/// Check if the range from call_start to arg_start is effectively single-line
/// after stripping whitespace (matching RuboCop's column_of behavior).
/// This is true when the text between call_start and arg_start, after stripping
/// leading/trailing whitespace, contains no newlines.
fn is_single_line_base_range(
    source: &SourceFile,
    call_start: usize,
    arg_start: usize,
    call_line: usize,
    arg_line: usize,
) -> bool {
    if call_line == arg_line {
        return true;
    }
    // For a range spanning exactly 2 lines (call on line N, arg on line N+1),
    // after stripping the trailing whitespace (spaces/newlines between the call's
    // opening paren and the arg), the result is typically single-line.
    // We check the actual source bytes.
    let bytes = source.as_bytes();
    if arg_start > call_start && arg_start <= bytes.len() {
        let range_bytes = &bytes[call_start..arg_start];
        // Strip trailing whitespace (spaces, tabs, newlines)
        let stripped = trim_end_whitespace(range_bytes);
        // Check if the stripped content contains any newline
        !stripped.contains(&b'\n')
    } else {
        call_line == arg_line
    }
}

fn trim_end_whitespace(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && matches!(bytes[end - 1], b' ' | b'\t' | b'\n' | b'\r') {
        end -= 1;
    }
    &bytes[..end]
}

/// Find the indentation of the previous non-blank, non-comment code line.
/// This matches RuboCop's `previous_code_line` behavior: the position of the
/// first non-whitespace character (`=~ /\S/`), counting both tabs and spaces
/// as 1 character each.
fn previous_code_line_indent(source: &SourceFile, line_number: usize) -> usize {
    let mut line_num = line_number;
    loop {
        if line_num <= 1 {
            return 0;
        }
        line_num -= 1;
        let line_bytes = source.lines().nth(line_num - 1).unwrap_or(b"");
        // Skip blank lines
        if line_bytes
            .iter()
            .all(|&b| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r')
        {
            continue;
        }
        // Skip comment lines (lines where first non-whitespace is #)
        // but NOT interpolation openings (#{...}) which appear in heredocs
        let mut after_ws = line_bytes
            .iter()
            .skip_while(|&&b| b == b' ' || b == b'\t')
            .copied();
        if after_ws.next() == Some(b'#') && after_ws.next() != Some(b'{') {
            continue;
        }
        // Count all leading whitespace characters (tabs and spaces),
        // matching RuboCop's `=~ /\S/` which treats each tab as 1 char
        return leading_whitespace_count(line_bytes);
    }
}

/// Count the number of leading whitespace characters (spaces and tabs).
/// Each tab counts as 1, matching Ruby's regex `=~ /\S/` behavior.
fn leading_whitespace_count(line: &[u8]) -> usize {
    line.iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count()
}

fn is_bare_operator(name: &str, has_regular_dot: bool) -> bool {
    method_identifier_predicates::is_operator_method(name.as_bytes()) && !has_regular_dot
}

fn is_comment_line_for_message(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#') && !trimmed.starts_with("#{")
}

fn is_setter_method(name: &str) -> bool {
    method_identifier_predicates::is_setter_method(name.as_bytes())
}

impl<'pr> Visit<'pr> for FirstArgVisitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        // Track call depth to know if we're the direct child of a splat
        self.call_depth += 1;
        let call_start_offset = node.location().start_offset();
        let name_bytes = node.name().as_slice();
        let name_str = std::str::from_utf8(name_bytes).unwrap_or("");
        let call_operator_loc = node.call_operator_loc();
        let has_regular_dot = call_operator_loc
            .as_ref()
            .is_some_and(|loc| loc.as_slice() == b".");

        // Check this call node for first argument indentation
        self.check_call(
            call_start_offset,
            node.message_loc(),
            node.opening_loc(),
            call_operator_loc,
            node.arguments(),
            CallMetadata {
                name: name_str,
                has_attached_block: node.block().and_then(|b| b.as_block_node()).is_some(),
            },
        );

        // Determine if this call is parenthesized and eligible for being a
        // "parent call" context for inner calls
        let is_parenthesized = node.opening_loc().is_some_and(|loc| loc.as_slice() == b"(");
        let is_eligible =
            !is_bare_operator(name_str, has_regular_dot) && !is_setter_method(name_str);

        // Collect argument start offsets
        let arg_start_offsets: Vec<usize> = node
            .arguments()
            .map(|args| {
                args.arguments()
                    .iter()
                    .map(|arg| arg.location().start_offset())
                    .collect()
            })
            .unwrap_or_default();

        let parent_info = ParentCallInfo {
            is_parenthesized,
            arg_start_offsets,
            is_eligible,
        };

        self.parent_call_stack.push(parent_info);
        ruby_prism::visit_call_node(self, node);
        self.parent_call_stack.pop();
        self.call_depth -= 1;
    }

    fn visit_super_node(&mut self, node: &ruby_prism::SuperNode<'pr>) {
        let call_start_offset = node.location().start_offset();

        // Check this super call for first argument indentation
        self.check_call(
            call_start_offset,
            Some(node.keyword_loc()),
            node.lparen_loc(),
            None,
            node.arguments(),
            CallMetadata {
                name: "super",
                has_attached_block: false,
            },
        );

        // super() is NOT eligible as a parent for special_inner_call checks.
        // RuboCop's eligible_method_call? uses `(send _ !:[]= ...)` which only
        // matches :send nodes, not :super nodes. So inner calls inside super()
        // should use previous_code_line_indent, not special inner call logic.
        let is_parenthesized = node.lparen_loc().is_some();

        let arg_start_offsets: Vec<usize> = node
            .arguments()
            .map(|args| {
                args.arguments()
                    .iter()
                    .map(|arg| arg.location().start_offset())
                    .collect()
            })
            .unwrap_or_default();

        let parent_info = ParentCallInfo {
            is_parenthesized,
            arg_start_offsets,
            is_eligible: false,
        };

        self.parent_call_stack.push(parent_info);
        ruby_prism::visit_super_node(self, node);
        self.parent_call_stack.pop();
    }

    fn visit_splat_node(&mut self, node: &ruby_prism::SplatNode<'pr>) {
        // Push splat offset when entering a splat (*args)
        self.splat_offsets.push(node.location().start_offset());
        // Manually visit the expression (the CallNode being splatted)
        // Increment splat_expr_depth so we know we're at depth 1 inside the splat
        self.splat_expr_depth += 1;
        if let Some(expr) = node.expression() {
            self.visit(&expr);
        }
        self.splat_expr_depth -= 1;
        self.splat_offsets.pop();
    }

    fn visit_assoc_splat_node(&mut self, node: &ruby_prism::AssocSplatNode<'pr>) {
        // Push assoc splat offset when entering an assoc splat (**kwargs)
        self.splat_offsets.push(node.location().start_offset());
        ruby_prism::visit_assoc_splat_node(self, node);
        self.splat_offsets.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        FirstArgumentIndentation,
        "cops/layout/first_argument_indentation"
    );

    #[test]
    fn args_on_same_line_ignored() {
        let source = b"foo(1, 2, 3)\n";
        let diags = run_cop_full(&FirstArgumentIndentation, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn chained_call_same_line_args_ignored() {
        // Chained call where arg is on same line as .method — should not flag
        let source = b"params\n  .require(:domain_block)\n  .slice(*PERMITTED)\n";
        let diags = run_cop_full(&FirstArgumentIndentation, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn special_inner_call_aligned_to_receiver() {
        // Inner call in parenthesized outer call — first arg aligned after receiver start + width
        let source = b"Conversation.create!(conversation_params.merge(\n                       contact_inbox_id: id\n                     ))\n";
        let diags = run_cop_full(&FirstArgumentIndentation, source);
        assert!(
            diags.is_empty(),
            "inner call with args aligned to receiver start should not be flagged, got: {:?}",
            diags
        );
    }

    #[test]
    fn special_inner_call_in_expect() {
        // expect(helper.generate_category_link(\n         portal_slug: 'x'\n       ))
        let source = b"expect(helper.generate_category_link(\n         portal_slug: 'portal_slug'\n       ))\n";
        let diags = run_cop_full(&FirstArgumentIndentation, source);
        assert!(
            diags.is_empty(),
            "inner call in expect() should not be flagged, got: {:?}",
            diags
        );
    }

    #[test]
    fn non_inner_call_still_flagged() {
        // Top-level call (not inside another call) with wrong indent
        let source = b"foo(\n      1\n)\n";
        let diags = run_cop_full(&FirstArgumentIndentation, source);
        assert_eq!(
            diags.len(),
            1,
            "non-inner call with wrong indent should be flagged"
        );
    }

    #[test]
    fn inner_call_with_attached_block_uses_previous_line_indent() {
        let source = b"include(foo.new(\n  bar\n) { |x| x })\n";
        let diags = run_cop_full(&FirstArgumentIndentation, source);
        assert!(
            diags.is_empty(),
            "inner call with attached block should not use special inner-call indentation, got: {:?}",
            diags
        );
    }

    #[test]
    fn plain_inner_call_without_block_still_uses_special_indent() {
        let source = b"include(foo.new(\n  bar\n))\n";
        let diags = run_cop_full(&FirstArgumentIndentation, source);
        assert_eq!(
            diags.len(),
            1,
            "plain inner call without block should still be flagged"
        );
    }

    #[test]
    fn inner_call_with_block_argument_still_uses_special_indent() {
        let source = b"      expect(foo.bar(\n        {\n          a: 1\n        }, &blk))\n";
        let diags = run_cop_full(&FirstArgumentIndentation, source);
        assert_eq!(
            diags.len(),
            1,
            "inner call with a block argument should still use special inner-call indentation"
        );
        assert_eq!(
            diags[0].message,
            "Indent the first argument one step more than `foo.bar(`."
        );
    }
}
