use ruby_prism::Visit;

use crate::cop::shared::method_identifier_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/Next: Use `next` to skip iteration instead of wrapping conditionals.
///
/// Key fixes applied:
/// - Check the LAST statement in block body, not just single-statement bodies
///   (RuboCop's `ends_with_condition?` logic). This was the main source of FN.
/// - Match RuboCop's line-based `MinBodyLength` check instead of counting
///   top-level statements. This fixes multiline single-statement and two-statement
///   bodies such as builder blocks, nested `do ... end` bodies, and multiline
///   hash literals.
/// - Match RuboCop's direct-child `if_else_children?` check instead of skipping
///   every outer guard whose body happens to contain a nested `if/unless ...
///   else`. In Prism, that means only skipping bodies whose sole statement is a
///   nested conditional with `else`; broader scanning caused real FN such as the
///   COSMOS-style trailing guard with an inner `if/else` among other statements,
///   while still allowing guarded bodies whose sole statement is a ternary.
/// - For terminal `unless` guards whose body is exactly one nested
///   `if`/`unless`, distinguish between single-statement and multi-statement
///   iteration bodies. RuboCop reports the nested conditional when the outer
///   `unless` is the entire loop body, but keeps the offense on the outer
///   `unless` when it is only the terminal statement in a larger body. The
///   previous unconditional inner-node remap caused the current FP/FN corpus
///   pairs by flagging the nested guard in cases where RuboCop keeps the outer
///   one.
/// - Added `while`/`until` loop support (RuboCop's `on_while`/`on_until`).
/// - Added `loop` and other missing enumerator methods (`inject`, `reduce`,
///   `find_index`, `map!`, `select!`, `reject!`).
/// - Added `each_*` prefix matching for dynamic enumerator methods.
/// - Removed `any?`/`none?` (not in RuboCop's ENUMERATOR_METHODS, caused FP).
/// - Removed `filter` (not in RuboCop's ENUMERATOR_METHODS, caused FP in
///   bluepotion `filter do` blocks).
/// - Fixed variant style divergence for `EnforcedStyle: always`: RuboCop's
///   `exit_body_type?` check (whether if_branch is break/return) is now
///   correctly applied based on style. In `skip_modifier_ifs`, modifier forms
///   are skipped via `allowed_modifier_if?` without consulting `exit_body_type?`.
///   In `always`, `exit_body_type?` is consulted for all forms - modifier forms
///   with exit bodies are not flagged, non-modifier forms with exit bodies are
///   not flagged (matching RuboCop behavior).
pub struct Next;

/// Check if a method name is an iteration method for Style/Next.
///
/// Uses the canonical `ENUMERATOR_METHODS` plus `each_*` prefix from rubocop-ast,
/// with `max_by`, `min_by`, `sort_by` added based on corpus investigation.
fn is_enumerator_method(name: &[u8]) -> bool {
    method_identifier_predicates::is_enumerator_method(name)
        || matches!(name, b"max_by" | b"min_by" | b"sort_by")
}

impl Cop for Next {
    fn name(&self) -> &'static str {
        "Style/Next"
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
        let style = config.get_str("EnforcedStyle", "skip_modifier_ifs");
        let min_body_length = config.get_usize("MinBodyLength", 3);
        let _allow_consecutive = config.get_bool("AllowConsecutiveConditionals", false);
        let mut visitor = NextVisitor {
            cop: self,
            source,
            style,
            min_body_length,
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct NextVisitor<'a> {
    cop: &'a Next,
    source: &'a SourceFile,
    style: &'a str,
    min_body_length: usize,
    diagnostics: Vec<Diagnostic>,
}

enum NestedConditional<'pr> {
    If(ruby_prism::IfNode<'pr>),
    Unless(ruby_prism::UnlessNode<'pr>),
}

impl NestedConditional<'_> {
    fn has_else(&self) -> bool {
        match self {
            Self::If(node) => node.subsequent().is_some(),
            Self::Unless(node) => node.else_clause().is_some(),
        }
    }

    fn has_keyword_else(&self) -> bool {
        match self {
            Self::If(node) => node.if_keyword_loc().is_some() && node.subsequent().is_some(),
            Self::Unless(node) => node.else_clause().is_some(),
        }
    }

    fn keyword_start_offset(&self) -> usize {
        match self {
            Self::If(node) => node
                .if_keyword_loc()
                .expect("non-modifier nested if should have `if` keyword")
                .start_offset(),
            Self::Unless(node) => node.keyword_loc().start_offset(),
        }
    }
}

impl NextVisitor<'_> {
    fn is_modifier_form(
        &self,
        keyword_loc: &ruby_prism::Location<'_>,
        statements: Option<ruby_prism::StatementsNode<'_>>,
    ) -> bool {
        statements.is_some_and(|stmts| stmts.location().start_offset() < keyword_loc.start_offset())
    }

    fn meets_min_body_length(
        &self,
        keyword_loc: &ruby_prism::Location<'_>,
        end_keyword_loc: Option<ruby_prism::Location<'_>>,
    ) -> bool {
        let Some(end_keyword_loc) = end_keyword_loc else {
            // Modifier forms do not have an `end`; RuboCop does not apply
            // MinBodyLength to them.
            return true;
        };

        let (keyword_line, _) = self.source.offset_to_line_col(keyword_loc.start_offset());
        let (end_line, _) = self
            .source
            .offset_to_line_col(end_keyword_loc.start_offset());
        end_line.saturating_sub(keyword_line) > self.min_body_length
    }

    fn is_exit_body(&self, statements: Option<ruby_prism::StatementsNode<'_>>) -> bool {
        let Some(statements) = statements else {
            return false;
        };

        let mut body = statements.body().iter();
        let Some(first_stmt) = body.next() else {
            return false;
        };

        body.next().is_none()
            && (first_stmt.as_break_node().is_some() || first_stmt.as_return_node().is_some())
    }

    fn single_nested_conditional<'pr>(
        &self,
        statements: Option<ruby_prism::StatementsNode<'pr>>,
    ) -> Option<NestedConditional<'pr>> {
        let statements = statements?;

        let mut body = statements.body().iter();
        let first_stmt = body.next()?;

        if body.next().is_some() {
            return None;
        }

        first_stmt
            .as_if_node()
            .map(NestedConditional::If)
            .or_else(|| first_stmt.as_unless_node().map(NestedConditional::Unless))
    }

    fn has_single_nested_conditional_with_else(
        &self,
        statements: Option<ruby_prism::StatementsNode<'_>>,
    ) -> bool {
        // RuboCop checks only direct child conditionals here. Prism always
        // wraps multi-statement branches in `StatementsNode`, so the equivalent
        // shape is a body whose sole statement is a nested conditional with
        // `else`.
        self.single_nested_conditional(statements)
            .is_some_and(|nested| nested.has_keyword_else())
    }

    fn check_block_body(&mut self, body: &ruby_prism::Node<'_>) {
        let stmts = match body.as_statements_node() {
            Some(s) => s,
            None => return,
        };

        let body_stmts: Vec<_> = stmts.body().iter().collect();
        if body_stmts.is_empty() {
            return;
        }
        let single_statement_body = body_stmts.len() == 1;

        // RuboCop checks if the LAST statement is an if/unless (ends_with_condition?)
        let stmt = &body_stmts[body_stmts.len() - 1];

        // Check for if/unless that wraps the entire block body
        if let Some(if_node) = stmt.as_if_node() {
            // Skip if it has an else branch
            if if_node.subsequent().is_some() {
                return;
            }

            // Skip modifier ifs if style is skip_modifier_ifs
            let Some(kw_loc) = if_node.if_keyword_loc() else {
                return;
            };
            let if_statements = if_node.statements();
            let is_modifier = self.is_modifier_form(&kw_loc, if_statements);

            if self.style == "skip_modifier_ifs" && is_modifier {
                return;
            }

            // RuboCop's exit_body_type? check is applied after allowed_modifier_if?
            // returns false. In always style, allowed_modifier_if? returns false
            // for modifier forms, so exit_body_type? IS consulted. We should check
            // is_exit_body for: always style (any form), or non-modifier forms.
            if (self.style == "always" || !is_modifier) && self.is_exit_body(if_node.statements()) {
                return;
            }

            if self.has_single_nested_conditional_with_else(if_node.statements()) {
                return;
            }

            if !self.meets_min_body_length(&kw_loc, if_node.end_keyword_loc()) {
                return;
            }

            let (line, column) = self.source.offset_to_line_col(kw_loc.start_offset());
            self.diagnostics.push(self.cop.diagnostic(
                self.source,
                line,
                column,
                "Use `next` to skip iteration.".to_string(),
            ));
        } else if let Some(unless_node) = stmt.as_unless_node() {
            // Skip if it has an else branch
            if unless_node.else_clause().is_some() {
                return;
            }

            // Skip modifier unless if style is skip_modifier_ifs
            let kw_loc = unless_node.keyword_loc();
            let is_modifier = self.is_modifier_form(&kw_loc, unless_node.statements());
            if self.style == "skip_modifier_ifs" && is_modifier {
                return;
            }

            // RuboCop's exit_body_type? check is applied after allowed_modifier_if?
            // returns false. In always style, allowed_modifier_if? returns false
            // for modifier forms, so exit_body_type? IS consulted. We should check
            // is_exit_body for: always style (any form), or non-modifier forms.
            if (self.style == "always" || !is_modifier)
                && self.is_exit_body(unless_node.statements())
            {
                return;
            }

            if self.has_single_nested_conditional_with_else(unless_node.statements()) {
                return;
            }

            if !self.meets_min_body_length(&kw_loc, unless_node.end_keyword_loc()) {
                return;
            }

            // RuboCop only remaps the offense to the nested conditional when
            // the outer `unless` is the ENTIRE iteration body. If other
            // top-level statements precede the terminal `unless`, keep the
            // offense on the outer guard.
            let start_offset = if single_statement_body {
                self.single_nested_conditional(unless_node.statements())
                    .filter(|nested| !nested.has_else())
                    .map_or_else(
                        || kw_loc.start_offset(),
                        |nested| nested.keyword_start_offset(),
                    )
            } else {
                kw_loc.start_offset()
            };
            let (line, column) = self.source.offset_to_line_col(start_offset);
            self.diagnostics.push(self.cop.diagnostic(
                self.source,
                line,
                column,
                "Use `next` to skip iteration.".to_string(),
            ));
        }
    }
}

impl<'pr> Visit<'pr> for NextVisitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let method_bytes = node.name().as_slice();

        if is_enumerator_method(method_bytes) {
            if let Some(block) = node.block() {
                if let Some(block_node) = block.as_block_node() {
                    if let Some(body) = block_node.body() {
                        self.check_block_body(&body);
                    }
                }
            }
        }

        // Visit children
        if let Some(recv) = node.receiver() {
            self.visit(&recv);
        }
        if let Some(args) = node.arguments() {
            for arg in args.arguments().iter() {
                self.visit(&arg);
            }
        }
        if let Some(block) = node.block() {
            self.visit(&block);
        }
    }

    fn visit_for_node(&mut self, node: &ruby_prism::ForNode<'pr>) {
        if let Some(stmts) = node.statements() {
            self.check_block_body(&stmts.as_node());
        }
        // Visit children
        self.visit(&node.collection());
        if let Some(stmts) = node.statements() {
            self.visit(&stmts.as_node());
        }
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        if let Some(stmts) = node.statements() {
            self.check_block_body(&stmts.as_node());
        }
        // Visit children
        if let Some(stmts) = node.statements() {
            self.visit(&stmts.as_node());
        }
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        if let Some(stmts) = node.statements() {
            self.check_block_body(&stmts.as_node());
        }
        // Visit children
        if let Some(stmts) = node.statements() {
            self.visit(&stmts.as_node());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(Next, "cops/style/next");
}
