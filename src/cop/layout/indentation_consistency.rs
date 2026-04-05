use crate::cop::shared::access_modifier_predicates;
use crate::cop::shared::node_type::{
    BEGIN_NODE, CALL_NODE, CASE_MATCH_NODE, CASE_NODE, CLASS_NODE, DEF_NODE, ELSE_NODE, FOR_NODE,
    IF_NODE, IN_NODE, LAMBDA_NODE, MODULE_NODE, PARENTHESES_NODE, PRE_EXECUTION_NODE, PROGRAM_NODE,
    SINGLETON_CLASS_NODE, STATEMENTS_NODE, UNLESS_NODE, UNTIL_NODE, WHEN_NODE, WHILE_NODE,
};
use crate::cop::shared::util::begins_its_line;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Layout/IndentationConsistency checks that the body of each construct
/// (class, module, def, block, if, unless, case/when, while, until, for, begin)
/// uses consistent indentation. All statements within a body must start at
/// the same column. The `indented_internal_methods` style only applies to
/// class/module/block bodies, not to if/while/etc.
///
/// ## Corpus investigation (2026-04-02)
///
/// High-volume divergence came from five gaps:
/// 1. Bodies whose first child shares the opener line (`do line = __LINE__`) were
///    skipped entirely, missing later misaligned lines.
/// 2. Prism wraps `def`/`block` bodies with `rescue` in an implicit `BeginNode`.
///    The cop only handled direct `StatementsNode`, so both the main body and
///    rescue/ensure bodies were missed.
/// 3. `class << self` (`SingletonClassNode`) bodies were not checked at all.
/// 4. Normal-style access modifiers were treated like ordinary body children.
///    RuboCop ignores bare `private`/`protected`/`public`/`module_function`
///    for alignment and only uses a leading modifier as the base column when it
///    is indented deeper than the enclosing body.
/// 5. Top-level sibling statements were never checked because Prism dispatches
///    the file body through `ProgramNode`, not a standalone top-level
///    `StatementsNode`. This missed scripts where one top-level statement is
///    indented differently from the next.
/// 6. Files starting with a UTF-8 BOM made the first top-level statement appear
///    to start at column 1. That bogus base column cascaded into false positives
///    for every later top-level sibling (`require`, `module`, `class`, etc.).
///    This cop now derives display columns that ignore a leading BOM on line 1.
///
/// ## Corpus investigation (2026-04-04)
///
/// Three additional gaps were found and fixed:
/// 7. Prism's Visit trait calls `visit_else_node` directly (without going
///    through `visit`/`visit_branch_node_enter`) for `CaseNode`, `CaseMatchNode`,
///    `UnlessNode`, and `BeginNode`. The walker never dispatched the `ElseNode`
///    to the cop, so `case/else`, `unless/else`, and `begin/rescue/else` bodies
///    were never checked for consistency. Fixed by handling else clauses
///    explicitly in the parent node handlers.
/// 8. For blocks (`do..end`/`{..}`), `parent_column` was derived from the
///    `do`/`{` keyword offset, which can be very high on the line (e.g.,
///    `Class.new do` has `do` at column 15+). RuboCop uses the call
///    node's start column instead. This caused access modifier checks to
///    fail: `private` at column 11 was not > `do` at column 19, so
///    `base_column` was None, and single non-modifier children were skipped.
///    Fixed by handling blocks via the CallNode handler, using
///    `call_node.location().start_offset()` for parent column.
/// 9. Lines starting with `;` (e.g., `; _erbout.<<(...)`) have their
///    expression node starting at column 2+, while other statements on
///    the line start at column 0. RuboCop's `begins_its_line?` check
///    skips such nodes. Added the same check to avoid false positives.
///
/// ## Corpus investigation (2026-04-04, round 2)
///
/// Fixed 10 FN and 1 FP:
/// 10. Lambda bodies (`-> { }` / `-> do end`, Prism `LambdaNode`) were
///     never checked for indentation consistency. Added a handler using
///     `opening_loc` for the keyword line and the lambda's own start
///     (`->`) for the parent column, matching how blocks use the call
///     node's start.
/// 11. `BEGIN { }` blocks (Prism `PreExecutionNode`) were never checked.
///     Added a handler using `opening_loc` for the keyword line.
/// 12. `check_statements_consistency` (used for if/when/while/etc. bodies)
///     did not filter bare access modifiers. RuboCop's `on_begin` handler
///     uniformly filters them from all body types. A `private` inside an
///     `if` body at a different column than sibling methods was flagged as
///     inconsistent, but RuboCop ignores it. Fixed by adding the same
///     access modifier filtering to `check_statements_consistency`.
///
/// ## Corpus investigation (2026-04-05)
///
/// Fixed 2 FN (poolparty chef_client.rb lines 16-17):
/// 13. Parenthesized expressions spanning multiple lines `(expr\nexpr\nexpr)`
///     are `begin` nodes in the parser gem, checked by RuboCop's `on_begin`.
///     In Prism they are `ParenthesesNode`, which was not handled. Added a
///     handler to check consistency of statements inside parenthesized
///     expressions.
pub struct IndentationConsistency;

/// Check if a node is a bare access modifier call
/// (private, protected, public, module_function with no args).
fn is_bare_access_modifier(node: &ruby_prism::Node<'_>) -> bool {
    node.as_call_node()
        .is_some_and(|call| access_modifier_predicates::is_bare_access_modifier(&call))
}

impl IndentationConsistency {
    fn line_col_for(&self, source: &SourceFile, byte_offset: usize) -> (usize, usize) {
        let (line, col) = source.offset_to_line_col(byte_offset);

        if line == 1 && byte_offset >= UTF8_BOM.len() && source.as_bytes().starts_with(&UTF8_BOM) {
            return (line, col.saturating_sub(1));
        }

        (line, col)
    }

    fn end_line_for(&self, source: &SourceFile, node: &ruby_prism::Node<'_>) -> usize {
        let loc = node.location();
        let end_offset = loc.end_offset().saturating_sub(1);
        source.offset_to_line_col(end_offset).0
    }

    fn statements_from_body<'pr>(
        &self,
        body: ruby_prism::Node<'pr>,
    ) -> Option<ruby_prism::StatementsNode<'pr>> {
        if let Some(stmts) = body.as_statements_node() {
            return Some(stmts);
        }

        body.as_begin_node()
            .and_then(|begin_node| begin_node.statements())
    }

    fn base_column_for_normal_style(
        &self,
        source: &SourceFile,
        children: &[ruby_prism::Node<'_>],
        parent_column: Option<usize>,
    ) -> Option<usize> {
        let first_child = children.first()?;
        if !is_bare_access_modifier(first_child) {
            return None;
        }

        let (_, access_modifier_column) =
            self.line_col_for(source, first_child.location().start_offset());

        match parent_column {
            Some(parent_column) => {
                (access_modifier_column > parent_column).then_some(access_modifier_column)
            }
            None => Some(access_modifier_column),
        }
    }

    fn check_child_list_consistency(
        &self,
        source: &SourceFile,
        children: Vec<ruby_prism::Node<'_>>,
        kw_line: usize,
        parent_column: Option<usize>,
        indented_internal_methods: bool,
    ) -> Vec<Diagnostic> {
        if children.len() < 2 {
            return Vec::new();
        }

        if indented_internal_methods {
            return self.check_sections(source, &children);
        }

        let base_column = self.base_column_for_normal_style(source, &children, parent_column);
        let filtered_children: Vec<_> = children
            .into_iter()
            .filter(|child| !is_bare_access_modifier(child))
            .collect();

        self.check_flat(source, &filtered_children, kw_line, base_column)
    }

    fn check_body_consistency(
        &self,
        source: &SourceFile,
        keyword_offset: usize,
        body: Option<ruby_prism::Node<'_>>,
        indented_internal_methods: bool,
    ) -> Vec<Diagnostic> {
        self.check_body_consistency_with_parent(
            source,
            keyword_offset,
            keyword_offset,
            body,
            indented_internal_methods,
        )
    }

    fn check_body_consistency_with_parent(
        &self,
        source: &SourceFile,
        keyword_offset: usize,
        parent_offset: usize,
        body: Option<ruby_prism::Node<'_>>,
        indented_internal_methods: bool,
    ) -> Vec<Diagnostic> {
        let body = match body {
            Some(b) => b,
            None => return Vec::new(),
        };

        let stmts = match self.statements_from_body(body) {
            Some(s) => s,
            None => return Vec::new(),
        };

        let (kw_line, _) = source.offset_to_line_col(keyword_offset);
        let (_, parent_column) = self.line_col_for(source, parent_offset);

        self.check_child_list_consistency(
            source,
            stmts.body().iter().collect(),
            kw_line,
            Some(parent_column),
            indented_internal_methods,
        )
    }

    /// Check consistency of a StatementsNode body (used for if/unless/when/while/etc
    /// where we get Option<StatementsNode> directly rather than Option<Node>).
    fn check_statements_consistency(
        &self,
        source: &SourceFile,
        keyword_offset: usize,
        stmts: Option<ruby_prism::StatementsNode<'_>>,
    ) -> Vec<Diagnostic> {
        let stmts = match stmts {
            Some(s) => s,
            None => return Vec::new(),
        };

        let children: Vec<_> = stmts.body().iter().collect();
        if children.len() < 2 {
            return Vec::new();
        }

        let (kw_line, _) = source.offset_to_line_col(keyword_offset);
        let (_, parent_column) = self.line_col_for(source, keyword_offset);

        // RuboCop's on_begin handler uniformly filters bare access modifiers
        // from all body types (if, when, while, etc.), not just class/module.
        let base_column = self.base_column_for_normal_style(source, &children, Some(parent_column));
        let filtered_children: Vec<_> = children
            .into_iter()
            .filter(|child| !is_bare_access_modifier(child))
            .collect();

        self.check_flat(source, &filtered_children, kw_line, base_column)
    }

    /// Normal style: all children must have the same indentation.
    fn check_flat(
        &self,
        source: &SourceFile,
        children: &[ruby_prism::Node<'_>],
        kw_line: usize,
        base_column: Option<usize>,
    ) -> Vec<Diagnostic> {
        if children.is_empty() || (children.len() < 2 && base_column.is_none()) {
            return Vec::new();
        }

        let first_loc = children[0].location();
        let (first_line, first_col) = self.line_col_for(source, first_loc.start_offset());
        let expected_column = base_column.unwrap_or(first_col);

        let mut diagnostics = Vec::new();
        let mut prev_end_line = self.end_line_for(source, &children[0]);

        if first_line != kw_line
            && first_col != expected_column
            && begins_its_line(source, first_loc.start_offset())
        {
            diagnostics.push(self.diagnostic(
                source,
                first_line,
                first_col,
                "Inconsistent indentation detected.".to_string(),
            ));
        }

        for child in &children[1..] {
            let loc = child.location();
            let (child_line, child_col) = self.line_col_for(source, loc.start_offset());

            // Skip semicolon-separated statements on the same line as previous sibling
            if child_line == prev_end_line || child_line == kw_line {
                prev_end_line = self.end_line_for(source, child);
                continue;
            }
            prev_end_line = self.end_line_for(source, child);

            if child_col != expected_column && begins_its_line(source, loc.start_offset()) {
                diagnostics.push(self.diagnostic(
                    source,
                    child_line,
                    child_col,
                    "Inconsistent indentation detected.".to_string(),
                ));
            }
        }

        diagnostics
    }

    /// indented_internal_methods style: access modifiers act as section dividers.
    /// Consistency is checked only within each section.
    fn check_sections(
        &self,
        source: &SourceFile,
        children: &[ruby_prism::Node<'_>],
    ) -> Vec<Diagnostic> {
        // Split children into sections separated by bare access modifiers.
        // Each section's children must have consistent indentation within the section,
        // but different sections can have different indentation levels.
        let mut sections: Vec<Vec<&ruby_prism::Node<'_>>> = vec![vec![]];

        for child in children {
            if is_bare_access_modifier(child) {
                // Start a new section (the modifier itself is not checked)
                sections.push(vec![]);
            } else {
                sections.last_mut().unwrap().push(child);
            }
        }

        let mut diagnostics = Vec::new();

        for section in &sections {
            if section.len() < 2 {
                continue;
            }

            let first_loc = section[0].location();
            let (_, first_col) = self.line_col_for(source, first_loc.start_offset());
            let mut prev_end_line = self.end_line_for(source, section[0]);

            for child in &section[1..] {
                let loc = child.location();
                let (child_line, child_col) = self.line_col_for(source, loc.start_offset());

                // Skip semicolon-separated statements on same line as previous sibling
                if child_line == prev_end_line {
                    prev_end_line = self.end_line_for(source, child);
                    continue;
                }
                prev_end_line = self.end_line_for(source, child);

                if child_col != first_col && begins_its_line(source, loc.start_offset()) {
                    diagnostics.push(self.diagnostic(
                        source,
                        child_line,
                        child_col,
                        "Inconsistent indentation detected.".to_string(),
                    ));
                }
            }
        }

        diagnostics
    }
}

impl Cop for IndentationConsistency {
    fn name(&self) -> &'static str {
        "Layout/IndentationConsistency"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            BEGIN_NODE,
            CALL_NODE,
            CASE_MATCH_NODE,
            CASE_NODE,
            CLASS_NODE,
            DEF_NODE,
            ELSE_NODE,
            FOR_NODE,
            IF_NODE,
            IN_NODE,
            LAMBDA_NODE,
            MODULE_NODE,
            PARENTHESES_NODE,
            PRE_EXECUTION_NODE,
            PROGRAM_NODE,
            SINGLETON_CLASS_NODE,
            STATEMENTS_NODE,
            UNLESS_NODE,
            UNTIL_NODE,
            WHEN_NODE,
            WHILE_NODE,
        ]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "normal");
        let indented = style == "indented_internal_methods";

        if let Some(program_node) = node.as_program_node() {
            diagnostics.extend(self.check_child_list_consistency(
                source,
                program_node.statements().body().iter().collect(),
                0,
                None,
                indented,
            ));
            return;
        }

        if let Some(class_node) = node.as_class_node() {
            diagnostics.extend(self.check_body_consistency(
                source,
                class_node.class_keyword_loc().start_offset(),
                class_node.body(),
                indented,
            ));
            return;
        }

        if let Some(module_node) = node.as_module_node() {
            diagnostics.extend(self.check_body_consistency(
                source,
                module_node.module_keyword_loc().start_offset(),
                module_node.body(),
                indented,
            ));
            return;
        }

        if let Some(singleton_class_node) = node.as_singleton_class_node() {
            diagnostics.extend(self.check_body_consistency(
                source,
                singleton_class_node.class_keyword_loc().start_offset(),
                singleton_class_node.body(),
                indented,
            ));
            return;
        }

        if let Some(def_node) = node.as_def_node() {
            diagnostics.extend(self.check_body_consistency(
                source,
                def_node.def_keyword_loc().start_offset(),
                def_node.body(),
                false, // indented_internal_methods only applies to class/module bodies
            ));
            return;
        }

        // Blocks are handled through their parent CallNode so we can use
        // the call's start column (matching RuboCop's node.parent.source_range)
        // instead of the `do`/`{` keyword column for access modifier comparison.
        if let Some(call_node) = node.as_call_node() {
            if let Some(block_node) = call_node.block().and_then(|n| n.as_block_node()) {
                diagnostics.extend(self.check_body_consistency_with_parent(
                    source,
                    block_node.opening_loc().start_offset(),
                    call_node.location().start_offset(),
                    block_node.body(),
                    indented,
                ));
            }
            return;
        }

        // Lambda bodies (-> { } / -> do end)
        if let Some(lambda_node) = node.as_lambda_node() {
            diagnostics.extend(self.check_body_consistency_with_parent(
                source,
                lambda_node.opening_loc().start_offset(),
                lambda_node.location().start_offset(),
                lambda_node.body(),
                indented,
            ));
            return;
        }

        // BEGIN { } blocks (PreExecutionNode)
        if let Some(pre_exec_node) = node.as_pre_execution_node() {
            diagnostics.extend(self.check_statements_consistency(
                source,
                pre_exec_node.opening_loc().start_offset(),
                pre_exec_node.statements(),
            ));
            return;
        }

        // Parenthesized expressions: (expr\nexpr\nexpr)
        // In the parser gem these are `begin` nodes and RuboCop's on_begin
        // checks their children for consistency. Prism uses ParenthesesNode.
        if let Some(parens_node) = node.as_parentheses_node() {
            if let Some(stmts) = parens_node.body().and_then(|b| b.as_statements_node()) {
                diagnostics.extend(self.check_statements_consistency(
                    source,
                    parens_node.opening_loc().start_offset(),
                    Some(stmts),
                ));
            }
            return;
        }

        // if/elsif bodies (ternary has no if_keyword_loc, skip those)
        if let Some(if_node) = node.as_if_node() {
            if let Some(kw_loc) = if_node.if_keyword_loc() {
                diagnostics.extend(self.check_statements_consistency(
                    source,
                    kw_loc.start_offset(),
                    if_node.statements(),
                ));
            }
            return;
        }

        // unless bodies
        if let Some(unless_node) = node.as_unless_node() {
            diagnostics.extend(self.check_statements_consistency(
                source,
                unless_node.keyword_loc().start_offset(),
                unless_node.statements(),
            ));
            // Prism's visit_unless_node calls visit_else_node directly,
            // bypassing visit_branch_node_enter, so the walker never sees
            // the ElseNode. Handle it explicitly here.
            if let Some(else_clause) = unless_node.else_clause() {
                diagnostics.extend(self.check_statements_consistency(
                    source,
                    else_clause.else_keyword_loc().start_offset(),
                    else_clause.statements(),
                ));
            }
            return;
        }

        // else bodies (from if/elsif/case/etc.)
        if let Some(else_node) = node.as_else_node() {
            diagnostics.extend(self.check_statements_consistency(
                source,
                else_node.else_keyword_loc().start_offset(),
                else_node.statements(),
            ));
            return;
        }

        // case/when bodies
        if let Some(when_node) = node.as_when_node() {
            diagnostics.extend(self.check_statements_consistency(
                source,
                when_node.keyword_loc().start_offset(),
                when_node.statements(),
            ));
            return;
        }

        // case/in bodies (pattern matching)
        if let Some(in_node) = node.as_in_node() {
            diagnostics.extend(self.check_statements_consistency(
                source,
                in_node.in_loc().start_offset(),
                in_node.statements(),
            ));
            return;
        }

        // case/else — Prism's visit_case_node calls visit_else_node directly,
        // bypassing visit_branch_node_enter, so the walker never dispatches
        // the ElseNode. Handle it explicitly here.
        if let Some(case_node) = node.as_case_node() {
            if let Some(else_clause) = case_node.else_clause() {
                diagnostics.extend(self.check_statements_consistency(
                    source,
                    else_clause.else_keyword_loc().start_offset(),
                    else_clause.statements(),
                ));
            }
            return;
        }

        // case/in/else (pattern matching) — same Prism visitor issue
        if let Some(case_match_node) = node.as_case_match_node() {
            if let Some(else_clause) = case_match_node.else_clause() {
                diagnostics.extend(self.check_statements_consistency(
                    source,
                    else_clause.else_keyword_loc().start_offset(),
                    else_clause.statements(),
                ));
            }
            return;
        }

        // while bodies
        if let Some(while_node) = node.as_while_node() {
            diagnostics.extend(self.check_statements_consistency(
                source,
                while_node.keyword_loc().start_offset(),
                while_node.statements(),
            ));
            return;
        }

        // until bodies
        if let Some(until_node) = node.as_until_node() {
            diagnostics.extend(self.check_statements_consistency(
                source,
                until_node.keyword_loc().start_offset(),
                until_node.statements(),
            ));
            return;
        }

        // for bodies
        if let Some(for_node) = node.as_for_node() {
            diagnostics.extend(self.check_statements_consistency(
                source,
                for_node.for_keyword_loc().start_offset(),
                for_node.statements(),
            ));
            return;
        }

        // begin bodies (only explicit begin blocks with begin keyword)
        if let Some(begin_node) = node.as_begin_node() {
            if let Some(kw_loc) = begin_node.begin_keyword_loc() {
                diagnostics.extend(self.check_statements_consistency(
                    source,
                    kw_loc.start_offset(),
                    begin_node.statements(),
                ));
            }

            let mut rescue_opt = begin_node.rescue_clause();
            while let Some(rescue_node) = rescue_opt {
                diagnostics.extend(self.check_statements_consistency(
                    source,
                    rescue_node.keyword_loc().start_offset(),
                    rescue_node.statements(),
                ));
                rescue_opt = rescue_node.subsequent();
            }

            if let Some(ensure_node) = begin_node.ensure_clause() {
                diagnostics.extend(self.check_statements_consistency(
                    source,
                    ensure_node.ensure_keyword_loc().start_offset(),
                    ensure_node.statements(),
                ));
            }

            // Prism's visit_begin_node calls visit_else_node directly,
            // bypassing visit_branch_node_enter. Handle else clause here.
            if let Some(else_clause) = begin_node.else_clause() {
                diagnostics.extend(self.check_statements_consistency(
                    source,
                    else_clause.else_keyword_loc().start_offset(),
                    else_clause.statements(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        IndentationConsistency,
        "cops/layout/indentation_consistency"
    );

    #[test]
    fn single_statement_body() {
        let source = b"def foo\n  x = 1\nend\n";
        let diags = run_cop_full(&IndentationConsistency, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn enforced_style_is_read() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("indented_internal_methods".into()),
            )]),
            ..CopConfig::default()
        };
        // In indented_internal_methods, methods in the same section before any
        // access modifier must be consistent. Two defs at different indentation
        // in the same section are flagged.
        let src = b"class Foo\n  def bar; end\n    def baz; end\nend\n";
        let diags = run_cop_full_with_config(&IndentationConsistency, src, config);
        assert!(
            !diags.is_empty(),
            "indented_internal_methods should still flag inconsistency within a section"
        );
    }

    #[test]
    fn checks_top_level_program_statements() {
        let source = b" require 'ostruct'\n\nmodule ClinicFinder\n  module Modules\n    module GestationHelper; end\n  end\nend\n";
        let diags = run_cop_full(&IndentationConsistency, source);

        assert_eq!(diags.len(), 1, "expected one top-level indentation offense");
        assert_eq!(diags[0].location.line, 3);
        assert_eq!(diags[0].location.column, 0);
        assert_eq!(diags[0].message, "Inconsistent indentation detected.");
    }

    #[test]
    fn bom_prefixed_top_level_statements_no_offense() {
        let source = b"\xef\xbb\xbfrequire 'colorize'\nrequire 'tmpdir'\n\nmodule Dryrun\nend\n";
        let diags = run_cop_full(&IndentationConsistency, source);

        assert!(
            diags.is_empty(),
            "BOM-prefixed top-level statements should not fire: {:?}",
            diags
        );
    }

    #[test]
    fn ignores_module_function_when_checking_block_body_consistency() {
        let source =
            b"m = Module.new do\n    module_function\n\n  def foo; end\n\n  def bar; end\nend\n";
        let diags = run_cop_full(&IndentationConsistency, source);

        assert!(
            diags.is_empty(),
            "module_function should not affect block body indentation: {:?}",
            diags
        );
    }

    #[test]
    fn indented_internal_methods_allows_extra_indent_after_private() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("indented_internal_methods".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"class Foo\n  def bar\n  end\n\n  private\n\n    def baz\n    end\n\n    def qux\n    end\nend\n";
        let diags = run_cop_full_with_config(&IndentationConsistency, src, config);
        assert!(
            diags.is_empty(),
            "indented_internal_methods should allow extra indent after private: {:?}",
            diags
        );
    }

    #[test]
    fn indented_internal_methods_flags_inconsistency_within_private_section() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("indented_internal_methods".into()),
            )]),
            ..CopConfig::default()
        };
        // Two methods after private at different indentation levels
        let src =
            b"class Foo\n  private\n\n    def bar\n    end\n\n      def baz\n      end\nend\n";
        let diags = run_cop_full_with_config(&IndentationConsistency, src, config);
        assert!(
            !diags.is_empty(),
            "should flag inconsistency within private section"
        );
    }
}
