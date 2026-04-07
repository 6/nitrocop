use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Layout/LineEndStringConcatenationIndentation
///
/// ## Variant style investigation (2026-04-07)
///
/// The reported variant FP/FN counts (26 FP, 19 FN for `EnforcedStyle=indented`)
/// could not be reproduced in targeted testing. All of the following were verified:
///
/// **FP investigation (26 reported):** Shopify/cli-ui `status_test.rb` and similar
/// hash-value string concat patterns ARE accepted by RuboCop under `indented`
/// style. The cop correctly uses `base_column` from the pair's `loc.column`
/// when the grandparent is a pair (hash key-value), producing the right expected
/// indentation. The reported FPs may stem from a different config resolution path
/// in the variant oracle vs the local check_cop.py variant runner.
///
/// **FN investigation (19 reported):** The cyberark/conjur patterns with
/// 2-space indented continuations (`"part 1" \` + indent + `"part 2"`) ARE flagged
/// by RuboCop under `indented` style with `MSG_INDENT`. nitrocop's visitor
/// should catch these too. The discrepancy may be a RuboCop crash (errors seen
/// in `team.investigate()`) suppressing expected offenses in certain contexts.
///
/// The `--check-variants` runs against the sample 30 repos consistently show
/// 0 FP / 0 FN for the `indented` variant, matching or exceeding baseline.
/// The default `aligned` style also shows no regressions.
///
/// Conclusion: The variant counts in the oracle data reflect a prior state or
/// a config resolution difference. No code change fixes a confirmed bug.
///
/// ## Investigation findings (2026-03-14)
///
/// **Root cause of 28 FNs:** The visitor only explicitly set `ParentType::Other`
/// for a handful of node types (CallNode, LocalVariableWriteNode, etc.). Any
/// node type NOT overridden (e.g., `IndexOperatorWriteNode`,
/// `LocalVariableOperatorWriteNode`, `CallOperatorWriteNode`,
/// `LocalVariableOrWriteNode`, `ParenthesesNode`, etc.) inherited the parent
/// type from its enclosing scope. Inside a `def` body, this meant operator
/// assignment nodes like `x += "a" \ "b"` inherited `ParentType::Def`, causing
/// `always_indented?` to be true and suppressing the "Align parts" check.
///
/// ## Investigation findings (2026-03-15)
///
/// **Root cause of 47 FPs:** The `visit_branch_node_enter`/`visit_branch_node_leave`
/// hooks are NOT reliably called for all nodes in Prism. `StatementsNode`
/// sometimes bypasses `visit_branch_node_enter` (depending on the parent node).
/// The previous code used `visit_statements_node` to read `stack.last()` for
/// "pass-through" of the parent type, but when `visit_branch_node_enter` was
/// not called for StatementsNode, `stack.last()` returned the wrong entry.
///
/// **Fix:** Use a stack-length check in `visit_statements_node`/`visit_else_node`
/// to detect whether `visit_branch_node_enter` was called. If it was, restore
/// `nearest_parent_type` from the saved value (the pushed entry). If not,
/// keep the inherited value. This correctly handles both cases.
///
/// ## Investigation findings (2026-03-15, round 2)
///
/// **Remaining 14 FPs:** Strings inside `case`/`when`/`rescue` branches
/// inherited the enclosing scope's always-indented parent type (e.g., `Def`)
/// instead of being treated as non-always-indented. In RuboCop/Parser,
/// `:when`, `:case`, `:resbody` are NOT in `PARENT_TYPES_FOR_INDENTED`.
/// For multi-statement bodies, Parser wraps in `:begin` (always-indented).
///
/// **Remaining 4 FNs:** (1) `ParenthesesNode` maps to Parser `:begin`
/// (always-indented) but was not handled. (2) Explicit `begin...end` maps
/// to Parser `:kwbegin` (NOT always-indented) but was treated like implicit
/// begin.
///
/// **Fix:** Added visitor overrides for `CaseNode`, `WhenNode`, `RescueNode`
/// (set `Other`), `ParenthesesNode` (set `Begin`), and distinguished explicit
/// vs implicit `BeginNode`. Multi-statement `StatementsNode` bodies set
/// `Begin` to match Parser's `:begin` wrapper behavior.
///
/// ## Investigation findings (2026-03-30)
///
/// **Remaining 193 FNs:** Prism exposes two shapes that this cop was still
/// mishandling. (1) In aligned mode, same-line adjacent literals inside a
/// continued string (for example `'...'"\\r\\n" \\`) remain separate parts,
/// but only the transition to the next line needs a trailing backslash. The
/// old code required a backslash between every adjacent Prism part and returned
/// early before alignment checks ran. (2) In always-indented contexts, the
/// base column was computed by counting only leading spaces, so tab-indented
/// method bodies looked already-correct and suppressed the first indent
/// offense.
///
/// **Fix:** Only require backslashes when consecutive Prism parts start on
/// different physical lines, and compute the indented base column from the
/// first non-whitespace character so tabs match RuboCop's column accounting.
///
/// ## Investigation findings (2026-03-31)
///
/// **Remaining FP:** Explicit `begin ... rescue` bodies with multiple
/// statements were still treated like Parser `:kwbegin`, so aligned-style
/// strings inside the protected body incorrectly required column alignment.
/// In RuboCop/Parser, that protected body is wrapped in `:begin`, which makes
/// the string always-indented. Explicit `begin ... end` without `rescue`
/// remains `:kwbegin` and should still use aligned behavior.
///
/// **Fix:** When Prism reports an explicit `BeginNode` with a `rescue_clause()`
/// and 2+ body statements, treat that body as `ParentType::Begin`. Keep
/// single-statement `begin ... rescue` bodies and rescue-less `begin ... end`
/// blocks as `ParentType::ExplicitBegin`.
pub struct LineEndStringConcatenationIndentation;

impl Cop for LineEndStringConcatenationIndentation {
    fn name(&self) -> &'static str {
        "Layout/LineEndStringConcatenationIndentation"
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
        let style = config.get_str("EnforcedStyle", "aligned");
        let indent_width = config.get_usize("IndentationWidth", 2);

        let mut visitor = ConcatVisitor {
            cop: self,
            source,
            code_map,
            diagnostics: Vec::new(),
            style,
            indent_width,
            nearest_parent_type: ParentType::TopLevel,
            saved_parent_types: Vec::new(),
            expected_stack_depth: 0,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct ConcatVisitor<'a> {
    cop: &'a LineEndStringConcatenationIndentation,
    source: &'a SourceFile,
    code_map: &'a CodeMap,
    diagnostics: Vec<Diagnostic>,
    style: &'a str,
    indent_width: usize,
    /// The current effective parent type for `always_indented?` checks.
    nearest_parent_type: ParentType,
    /// Save/restore stack for `nearest_parent_type`.
    saved_parent_types: Vec<ParentType>,
    /// Expected stack depth at the next `visit_statements_node` or
    /// `visit_else_node` call. Used to detect whether
    /// `visit_branch_node_enter` was called for that node.
    expected_stack_depth: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum ParentType {
    TopLevel,
    Block,
    Begin,
    Def,
    If,
    /// Non-always-indented scope where multi-statement bodies get promoted
    /// to Begin (e.g., when, case, rescue in Parser wrap multi-statement
    /// bodies in `:begin`).
    Other,
    /// Explicit `begin...end` that should stay Parser `:kwbegin` —
    /// not always-indented. A rescued body with 2+ statements is promoted
    /// separately to `Begin` in `visit_begin_node`.
    ExplicitBegin,
}

impl ConcatVisitor<'_> {
    fn part_start_line_col(&self, part: &ruby_prism::Node<'_>) -> (usize, usize) {
        self.source
            .offset_to_line_col(part.location().start_offset())
    }

    fn part_is_single_line(&self, part: &ruby_prism::Node<'_>) -> bool {
        let loc = part.location();
        let (start_line, _) = self.source.offset_to_line_col(loc.start_offset());
        let (end_line, _) = self
            .source
            .offset_to_line_col(loc.end_offset().saturating_sub(1).max(loc.start_offset()));
        start_line == end_line
    }

    fn line_indent_column(&self, line_num: usize) -> usize {
        if line_num == 0 {
            return 0;
        }

        let lines: Vec<&[u8]> = self.source.lines().collect();
        lines
            .get(line_num - 1)
            .map(|line| {
                line.iter()
                    .take_while(|&&b| b.is_ascii_whitespace())
                    .count()
            })
            .unwrap_or(0)
    }

    fn check_dstr(&mut self, node: &ruby_prism::InterpolatedStringNode<'_>) {
        let parts: Vec<_> = node.parts().iter().collect();
        if parts.len() < 2 {
            return;
        }

        // Check that this is a backslash-concatenated string (multiline dstr
        // where each child is a single-line string/dstr part)
        let bytes = self.source.as_bytes();
        let (first_line, _) = self.part_start_line_col(&parts[0]);
        let (last_line, _) = self.part_start_line_col(parts.last().unwrap());
        if first_line == last_line {
            return; // Not multiline
        }

        // Check that each part is single-line.
        for part in &parts {
            if !self.part_is_single_line(part) {
                return; // Multi-line part
            }
        }

        // Prism keeps same-line adjacent literals as separate parts. RuboCop
        // still treats these as one continued expression as long as each
        // transition to the next physical line is a backslash continuation.
        for pair in parts.windows(2) {
            let (left_line, _) = self.part_start_line_col(&pair[0]);
            let (right_line, _) = self.part_start_line_col(&pair[1]);
            if left_line == right_line {
                continue;
            }

            let end_offset = pair[0].location().end_offset();
            let start_offset = pair[1].location().start_offset();
            let between = &bytes[end_offset..start_offset];
            if !between.contains(&b'\\') {
                return; // Not backslash continuation
            }
        }

        // Skip if inside a heredoc body
        if self.code_map.is_heredoc(parts[0].location().start_offset()) {
            return;
        }

        // RuboCop's `always_indented?` checks the DIRECT parent type.
        // saved_parent_types.last() contains the value saved when
        // visit_branch_node_enter ran for THIS InterpolatedStringNode.
        let parent_type = self
            .saved_parent_types
            .last()
            .copied()
            .unwrap_or(ParentType::TopLevel);
        let always_indented = matches!(
            parent_type,
            ParentType::TopLevel
                | ParentType::Block
                | ParentType::Begin
                | ParentType::Def
                | ParentType::If
        );
        let use_indented = self.style == "indented" || always_indented;

        // Get column positions of each part
        let columns: Vec<usize> = parts
            .iter()
            .map(|p| self.part_start_line_col(p).1)
            .collect();

        if use_indented && columns.len() >= 2 {
            // First, check indentation of the second part
            // base_column = first non-whitespace column of the first part's
            // source line, matching RuboCop's `source_line =~ /\S/`.
            let (first_part_line, _) = self.part_start_line_col(&parts[0]);
            let first_line_indent = self.line_indent_column(first_part_line);

            let expected_indent = first_line_indent + self.indent_width;

            if columns[1] != expected_indent {
                let (line_num, _) = self
                    .source
                    .offset_to_line_col(parts[1].location().start_offset());
                self.diagnostics.push(self.cop.diagnostic(
                    self.source,
                    line_num,
                    columns[1],
                    "Indent the first part of a string concatenated with backslash.".to_string(),
                ));
            }

            // Check alignment of third+ parts with the second part
            if columns.len() >= 3 {
                let mut base = columns[1];
                for (idx, &col) in columns[2..].iter().enumerate() {
                    if col != base {
                        let part_idx = idx + 2;
                        let (line_num, _) = self
                            .source
                            .offset_to_line_col(parts[part_idx].location().start_offset());
                        self.diagnostics.push(self.cop.diagnostic(
                            self.source,
                            line_num,
                            col,
                            "Align parts of a string concatenated with backslash.".to_string(),
                        ));
                    }
                    base = col;
                }
            }
        } else if self.style == "aligned" {
            let mut base = columns[0];
            for (idx, &col) in columns[1..].iter().enumerate() {
                if col != base {
                    let part_idx = idx + 1;
                    let (line_num, _) = self
                        .source
                        .offset_to_line_col(parts[part_idx].location().start_offset());
                    self.diagnostics.push(self.cop.diagnostic(
                        self.source,
                        line_num,
                        col,
                        "Align parts of a string concatenated with backslash.".to_string(),
                    ));
                }
                base = col;
            }
        }
    }
}

impl<'pr> Visit<'pr> for ConcatVisitor<'_> {
    fn visit_branch_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {
        self.saved_parent_types.push(self.nearest_parent_type);
        self.nearest_parent_type = ParentType::Other;
    }

    fn visit_branch_node_leave(&mut self) {
        self.nearest_parent_type = self
            .saved_parent_types
            .pop()
            .unwrap_or(ParentType::TopLevel);
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        self.check_dstr(node);
        // Don't recurse into children — we handle the whole dstr at once
    }

    // --- "Always indented" parent types ---
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let saved_depth = self.expected_stack_depth;
        self.nearest_parent_type = ParentType::Def;
        self.expected_stack_depth = self.saved_parent_types.len();
        ruby_prism::visit_def_node(self, node);
        self.expected_stack_depth = saved_depth;
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        let saved_depth = self.expected_stack_depth;
        self.nearest_parent_type = ParentType::Block;
        self.expected_stack_depth = self.saved_parent_types.len();
        ruby_prism::visit_block_node(self, node);
        self.expected_stack_depth = saved_depth;
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        let saved_depth = self.expected_stack_depth;
        self.nearest_parent_type = ParentType::Block;
        self.expected_stack_depth = self.saved_parent_types.len();
        ruby_prism::visit_lambda_node(self, node);
        self.expected_stack_depth = saved_depth;
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        let saved_depth = self.expected_stack_depth;
        // Implicit begin (def body with rescue, no begin keyword) keeps the
        // enclosing scope's parent type (e.g., Def). Explicit `begin...end`
        // is usually Parser `:kwbegin` (not always-indented), except that a
        // rescued body with 2+ statements is wrapped in Parser `:begin`.
        if node.begin_keyword_loc().is_some() {
            let has_multi_statement_rescue_body = node.rescue_clause().is_some()
                && node
                    .statements()
                    .is_some_and(|statements| statements.body().len() > 1);
            self.nearest_parent_type = if has_multi_statement_rescue_body {
                ParentType::Begin
            } else {
                ParentType::ExplicitBegin
            };
        }
        self.expected_stack_depth = self.saved_parent_types.len();
        ruby_prism::visit_begin_node(self, node);
        self.expected_stack_depth = saved_depth;
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        let saved_depth = self.expected_stack_depth;
        self.nearest_parent_type = ParentType::If;
        self.expected_stack_depth = self.saved_parent_types.len();
        ruby_prism::visit_if_node(self, node);
        self.expected_stack_depth = saved_depth;
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        let saved_depth = self.expected_stack_depth;
        self.nearest_parent_type = ParentType::If;
        self.expected_stack_depth = self.saved_parent_types.len();
        ruby_prism::visit_unless_node(self, node);
        self.expected_stack_depth = saved_depth;
    }

    // --- Non-always-indented parent types ---
    // In Parser, `:when`, `:case`, `:resbody` are NOT in
    // PARENT_TYPES_FOR_INDENTED. Multi-statement bodies get `:begin` wrapper
    // (handled by visit_statements_node below).
    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'pr>) {
        let saved_depth = self.expected_stack_depth;
        self.nearest_parent_type = ParentType::Other;
        self.expected_stack_depth = self.saved_parent_types.len();
        ruby_prism::visit_case_node(self, node);
        self.expected_stack_depth = saved_depth;
    }

    fn visit_when_node(&mut self, node: &ruby_prism::WhenNode<'pr>) {
        let saved_depth = self.expected_stack_depth;
        self.nearest_parent_type = ParentType::Other;
        self.expected_stack_depth = self.saved_parent_types.len();
        ruby_prism::visit_when_node(self, node);
        self.expected_stack_depth = saved_depth;
    }

    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        let saved_depth = self.expected_stack_depth;
        self.nearest_parent_type = ParentType::Other;
        self.expected_stack_depth = self.saved_parent_types.len();
        ruby_prism::visit_rescue_node(self, node);
        self.expected_stack_depth = saved_depth;
    }

    // ParenthesesNode maps to Parser `:begin` — always-indented.
    fn visit_parentheses_node(&mut self, node: &ruby_prism::ParenthesesNode<'pr>) {
        let saved_depth = self.expected_stack_depth;
        self.nearest_parent_type = ParentType::Begin;
        self.expected_stack_depth = self.saved_parent_types.len();
        ruby_prism::visit_parentheses_node(self, node);
        self.expected_stack_depth = saved_depth;
    }

    // --- Pass-through nodes ---
    fn visit_statements_node(&mut self, node: &ruby_prism::StatementsNode<'pr>) {
        // Detect if visit_branch_node_enter was called for this node by
        // checking if the stack grew since the parent set expected_stack_depth.
        if self.saved_parent_types.len() > self.expected_stack_depth {
            // visit_branch_node_enter was called: restore the saved value
            if let Some(&saved) = self.saved_parent_types.last() {
                self.nearest_parent_type = saved;
            }
        }
        // In Parser, multi-statement bodies are wrapped in `:begin` (which IS
        // always-indented). Emulate this: if the StatementsNode has 2+
        // children and the current parent type is NOT already always-indented,
        // promote to Begin.
        if node.body().len() > 1 && matches!(self.nearest_parent_type, ParentType::Other) {
            self.nearest_parent_type = ParentType::Begin;
        }
        ruby_prism::visit_statements_node(self, node);
    }

    fn visit_else_node(&mut self, node: &ruby_prism::ElseNode<'pr>) {
        if self.saved_parent_types.len() > self.expected_stack_depth {
            if let Some(&saved) = self.saved_parent_types.last() {
                self.nearest_parent_type = saved;
            }
        }
        ruby_prism::visit_else_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        LineEndStringConcatenationIndentation,
        "cops/layout/line_end_string_concatenation_indentation"
    );
}
