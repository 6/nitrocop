use crate::cop::shared::node_type::{
    CASE_MATCH_NODE, CASE_NODE, CLASS_NODE, IF_NODE, MODULE_NODE, SINGLETON_CLASS_NODE,
    UNLESS_NODE, UNTIL_NODE, WHILE_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Layout/EndAlignment: checks that `end` keywords are aligned with their opening keyword.
///
/// Investigation findings (2026-03-14):
/// - **5 FPs** from BOM (U+FEFF) at file start: the 3-byte UTF-8 BOM counted as 1 column,
///   making `module` appear at col 1 instead of col 0. Fixed by subtracting the BOM character
///   from keyword column when on line 1.
/// - **55 FNs** from missing node types:
///   - `UnlessNode`: Prism parses `unless` as a separate node type, not `IfNode`.
///   - `CaseMatchNode`: pattern matching `case/in` uses `CaseMatchNode`, not `CaseNode`.
///   - `SingletonClassNode`: `class << self` uses `SingletonClassNode`, not `ClassNode`.
///     All three were added to `interested_node_types` and handled in `check_node`.
///
/// ## Corpus investigation (2026-03-15)
///
/// Corpus oracle reported FP=0, FN=6 (all from rage-rb). Root cause was NOT
/// in this cop's logic but in `DisabledRanges::from_comments` in
/// `src/parse/directives.rs`: `# rubocop:enable all` only closed a disable
/// for the literal string "all", not individual per-cop disables
/// (`Layout/EndAlignment` etc.) that were opened by `# rubocop:disable`.
/// The rage-rb files had `# rubocop:disable Layout/EndAlignment` (line 10)
/// and `# rubocop:enable all` (line 170), but the enable didn't close the
/// per-cop disable, so all subsequent offenses were incorrectly suppressed.
/// Fixed in `directives.rs`: `enable all` now drains all open disables;
/// department enables now close both the department and its individual cops.
///
/// ## Variant style investigation (2026-04-08)
///
/// - `EnforcedStyleAlignWith: start_of_line` had large tab-indented divergence because
///   the expected column was computed by counting only ASCII spaces. RuboCop aligns to
///   the first non-whitespace character on the line, so `\tif ...` / `\tend` is valid
///   and mixed leading whitespace like `" \tif"` still expects column 2.
/// - `EnforcedStyleAlignWith: variable` was using broad line scans for `=`/`<<`, which
///   incorrectly treated nested expressions like `content = label || if ... end` as
///   assignment-aligned and missed real operator/send contexts like `model == if ... end`
///   and `raise Informative, if ... end`. Fixed by matching RuboCop's narrower context:
///   only align to an outer assignment/send when the conditional is the extracted RHS /
///   last-argument target after peeling call chains and grouping wrappers. Other same-line
///   parents like `return case`, hash pairs, `when ... then case`, and `foo || case`
///   still fall back to keyword alignment. `class`/`module` also stay keyword-aligned
///   under `variable`, while `case`/`case in` only align to an immediate same-line
///   send/assignment parent when Prism exposes a real `predicate`.
///
/// ## Variant fix round 2 (2026-04-08)
///
/// **variable style — 23 FP, 63 FN:**
/// - FPs: case nodes under `!!` (double-not) were treated as arguments of the `!`
///   send, causing alignment with the `!` position instead of keyword. Root cause:
///   the case-with-predicate check used `AncestorKind::Send` broadly but didn't
///   distinguish argument-of-send from receiver-of-send. Fixed by adding
///   `arguments_span` to `AncestorContext` and only aligning with parent sends when
///   the case's start offset falls within the arguments span.
/// - FNs: bare `case` (no predicate) under assignment (`var = case when ...`) was
///   always falling back to keyword alignment because `variable_context_start_offset`
///   returned `None` early for bare case. Fixed by removing the early return and
///   letting bare case fall through to the general rhs-matching logic, which correctly
///   detects the assignment parent via `extracted_rhs`.
///
/// **start_of_line style — 5 FN:**
/// - All from BOM (U+FEFF) at file start. RuboCop's `effective_column` subtracts 1
///   from columns on line 1 with BOM, making the start-of-line alignment position
///   effectively -1 (unreachable). Fixed by detecting when the alignment offset falls
///   within the BOM (offset < 3 on line 1 with BOM) and forcing a mismatch.
pub struct EndAlignment;

fn alignment_column(source: &SourceFile, offset: usize) -> usize {
    let (line, col) = source.offset_to_line_col(offset);
    if line == 1 {
        let bytes = source.as_bytes();
        if bytes.len() >= 3
            && bytes[0] == 0xEF
            && bytes[1] == 0xBB
            && bytes[2] == 0xBF
            && offset >= 3
        {
            return col.saturating_sub(1);
        }
    }
    col
}

fn start_of_line_alignment_offset(source: &SourceFile, offset: usize) -> usize {
    let bytes = source.as_bytes();
    let (line, _) = source.offset_to_line_col(offset);
    let mut pos = source.line_start_offset(line);
    while pos < bytes.len() && bytes[pos] != b'\n' && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        pos += 1;
    }
    pos
}

fn first_part_of_call_chain(mut node: ruby_prism::Node<'_>) -> ruby_prism::Node<'_> {
    while let Some(call) = node.as_call_node() {
        let Some(receiver) = call.receiver() else {
            break;
        };
        node = receiver;
    }
    node
}

fn unwrap_grouping(mut node: ruby_prism::Node<'_>) -> ruby_prism::Node<'_> {
    loop {
        if let Some(parentheses) = node.as_parentheses_node() {
            let Some(body) = parentheses.body() else {
                break;
            };
            let Some(stmts) = body.as_statements_node() else {
                break;
            };
            let body = stmts.body();
            if body.len() != 1 {
                break;
            }
            let Some(single) = body.iter().next() else {
                break;
            };
            node = single;
            continue;
        }

        if let Some(stmts) = node.as_statements_node() {
            let body = stmts.body();
            if body.len() != 1 {
                break;
            }
            let Some(single) = body.iter().next() else {
                break;
            };
            node = single;
            continue;
        }

        if let Some(begin_node) = node.as_begin_node() {
            if begin_node.begin_keyword_loc().is_some()
                || begin_node.rescue_clause().is_some()
                || begin_node.else_clause().is_some()
                || begin_node.ensure_clause().is_some()
            {
                break;
            }

            let Some(stmts) = begin_node.statements() else {
                break;
            };
            let body = stmts.body();
            if body.len() != 1 {
                break;
            }
            let Some(single) = body.iter().next() else {
                break;
            };
            node = single;
            continue;
        }

        break;
    }

    node
}

fn extracted_rhs<'pr>(node: &'pr ruby_prism::Node<'pr>) -> Option<ruby_prism::Node<'pr>> {
    if let Some(call) = node.as_call_node() {
        return call.arguments()?.arguments().last();
    }
    if let Some(asgn) = node.as_local_variable_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_local_variable_or_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_local_variable_and_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_local_variable_operator_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_instance_variable_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_instance_variable_or_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_instance_variable_and_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_instance_variable_operator_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_class_variable_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_class_variable_or_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_class_variable_and_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_class_variable_operator_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_global_variable_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_global_variable_or_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_global_variable_and_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_global_variable_operator_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_constant_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_constant_or_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_constant_and_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_constant_operator_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_constant_path_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_constant_path_or_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_constant_path_and_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_constant_path_operator_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_multi_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_call_or_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_call_and_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_call_operator_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_index_or_write_node() {
        return Some(asgn.value());
    }
    if let Some(asgn) = node.as_index_and_write_node() {
        return Some(asgn.value());
    }
    node.as_index_operator_write_node().map(|asgn| asgn.value())
}

#[derive(Clone, Copy)]
enum AncestorKind {
    Other,
    Send,
    AssignmentLike,
}

#[derive(Clone, Copy)]
struct AncestorContext {
    start_offset: usize,
    rhs_span: Option<(usize, usize)>,
    kind: AncestorKind,
    /// For Send (CallNode) ancestors, the span of the arguments list.
    /// Used to distinguish "case is argument of send" from "case is receiver of send".
    arguments_span: Option<(usize, usize)>,
}

#[derive(Clone, Copy)]
struct KeywordEndContext<'a> {
    kw_offset: usize,
    end_offset: usize,
    keyword: &'a str,
    style: &'a str,
}

struct AncestorFinder {
    target_span: (usize, usize),
    stack: Vec<AncestorContext>,
    found: Option<Vec<AncestorContext>>,
}

fn ancestor_kind(node: &ruby_prism::Node<'_>) -> AncestorKind {
    if node.as_call_node().is_some() {
        AncestorKind::Send
    } else if extracted_rhs(node).is_some() {
        AncestorKind::AssignmentLike
    } else {
        AncestorKind::Other
    }
}

impl<'pr> ruby_prism::Visit<'pr> for AncestorFinder {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        let node_span = (node.location().start_offset(), node.location().end_offset());
        if self.found.is_none() && node_span == self.target_span {
            self.found = Some(self.stack.clone());
        }

        let rhs_span = extracted_rhs(&node).map(|rhs| {
            let rhs = unwrap_grouping(first_part_of_call_chain(rhs));
            (rhs.location().start_offset(), rhs.location().end_offset())
        });

        let arguments_span = node.as_call_node().and_then(|call| {
            let args = call.arguments()?;
            Some((args.location().start_offset(), args.location().end_offset()))
        });

        self.stack.push(AncestorContext {
            start_offset: node.location().start_offset(),
            rhs_span,
            kind: ancestor_kind(&node),
            arguments_span,
        });
    }

    fn visit_branch_node_leave(&mut self) {
        self.stack.pop();
    }

    fn visit_leaf_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        let node_span = (node.location().start_offset(), node.location().end_offset());
        if self.found.is_none() && node_span == self.target_span {
            self.found = Some(self.stack.clone());
        }
    }
}

fn ancestors_for_node(
    parse_result: &ruby_prism::ParseResult<'_>,
    node: &ruby_prism::Node<'_>,
) -> Vec<AncestorContext> {
    let mut finder = AncestorFinder {
        target_span: (node.location().start_offset(), node.location().end_offset()),
        stack: Vec::new(),
        found: None,
    };
    finder.visit(&parse_result.node());
    finder.found.unwrap_or_default()
}

fn variable_context_start_offset(
    source: &SourceFile,
    ancestors: &[AncestorContext],
    node: &ruby_prism::Node<'_>,
    kw_offset: usize,
) -> Option<usize> {
    let (kw_line, _) = source.offset_to_line_col(kw_offset);
    let immediate_parent_start_offset = |predicate: fn(&AncestorContext) -> bool| {
        let parent = ancestors.last()?;
        let (parent_line, _) = source.offset_to_line_col(parent.start_offset);
        if parent_line == kw_line && predicate(parent) {
            Some(parent.start_offset)
        } else {
            None
        }
    };

    if node.as_class_node().is_some() || node.as_module_node().is_some() {
        return None;
    }

    if node.as_singleton_class_node().is_some() {
        return immediate_parent_start_offset(|parent| {
            matches!(parent.kind, AncestorKind::AssignmentLike)
        });
    }

    // For case/case_match nodes:
    // 1. If case is an argument of a parent send (not the receiver) on the same line,
    //    align with the parent send (matches RuboCop's `node.argument?` check).
    // 2. Otherwise, fall through to the general rhs-matching logic, which handles
    //    case as RHS of assignments (both bare case and case-with-predicate).
    // 3. If no parent matches, returns None → keyword alignment.
    let is_case = node.as_case_node().is_some() || node.as_case_match_node().is_some();
    if is_case {
        if let Some(parent) = ancestors.last() {
            if matches!(parent.kind, AncestorKind::Send) {
                let node_start = node.location().start_offset();
                let is_argument = parent.arguments_span.map_or(false, |(arg_start, arg_end)| {
                    node_start >= arg_start && node_start < arg_end
                });
                if is_argument {
                    let (parent_line, _) = source.offset_to_line_col(parent.start_offset);
                    if parent_line == kw_line {
                        return Some(parent.start_offset);
                    }
                    return None; // line break before keyword → keyword alignment
                }
            }
        }
        // Fall through to general rhs-matching logic (handles assignments, etc.)
    }

    let target_span = (node.location().start_offset(), node.location().end_offset());
    for parent in ancestors.iter().rev() {
        if parent.rhs_span == Some(target_span) {
            let (parent_line, _) = source.offset_to_line_col(parent.start_offset);
            if parent_line == kw_line {
                return Some(parent.start_offset);
            }
            return None;
        }
    }

    None
}

impl Cop for EndAlignment {
    fn name(&self) -> &'static str {
        "Layout/EndAlignment"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CASE_MATCH_NODE,
            CASE_NODE,
            CLASS_NODE,
            IF_NODE,
            MODULE_NODE,
            SINGLETON_CLASS_NODE,
            UNLESS_NODE,
            UNTIL_NODE,
            WHILE_NODE,
        ]
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
        let style = config.get_str("EnforcedStyleAlignWith", "keyword");
        if let Some(class_node) = node.as_class_node() {
            diagnostics.extend(self.check_keyword_end(
                source,
                node,
                parse_result,
                KeywordEndContext {
                    kw_offset: class_node.class_keyword_loc().start_offset(),
                    end_offset: class_node.end_keyword_loc().start_offset(),
                    keyword: "class",
                    style,
                },
            ));
            return;
        }

        if let Some(module_node) = node.as_module_node() {
            diagnostics.extend(self.check_keyword_end(
                source,
                node,
                parse_result,
                KeywordEndContext {
                    kw_offset: module_node.module_keyword_loc().start_offset(),
                    end_offset: module_node.end_keyword_loc().start_offset(),
                    keyword: "module",
                    style,
                },
            ));
            return;
        }

        if let Some(if_node) = node.as_if_node() {
            let kw_loc = match if_node.if_keyword_loc() {
                Some(loc) => loc,
                None => return,
            };
            // Only check top-level if/unless, not elsif
            let kw_slice = kw_loc.as_slice();
            if kw_slice != b"if" && kw_slice != b"unless" {
                return;
            }
            let end_kw_loc = match if_node.end_keyword_loc() {
                Some(loc) => loc,
                None => return,
            };
            let keyword = if kw_slice == b"if" { "if" } else { "unless" };
            diagnostics.extend(self.check_keyword_end(
                source,
                node,
                parse_result,
                KeywordEndContext {
                    kw_offset: kw_loc.start_offset(),
                    end_offset: end_kw_loc.start_offset(),
                    keyword,
                    style,
                },
            ));
            return;
        }

        if let Some(while_node) = node.as_while_node() {
            let kw_loc = while_node.keyword_loc();
            if let Some(end_loc) = while_node.closing_loc() {
                diagnostics.extend(self.check_keyword_end(
                    source,
                    node,
                    parse_result,
                    KeywordEndContext {
                        kw_offset: kw_loc.start_offset(),
                        end_offset: end_loc.start_offset(),
                        keyword: "while",
                        style,
                    },
                ));
                return;
            }
        }

        if let Some(until_node) = node.as_until_node() {
            let kw_loc = until_node.keyword_loc();
            if let Some(end_loc) = until_node.closing_loc() {
                diagnostics.extend(self.check_keyword_end(
                    source,
                    node,
                    parse_result,
                    KeywordEndContext {
                        kw_offset: kw_loc.start_offset(),
                        end_offset: end_loc.start_offset(),
                        keyword: "until",
                        style,
                    },
                ));
                return;
            }
        }

        if let Some(case_node) = node.as_case_node() {
            let kw_loc = case_node.case_keyword_loc();
            let end_loc = case_node.end_keyword_loc();
            diagnostics.extend(self.check_keyword_end(
                source,
                node,
                parse_result,
                KeywordEndContext {
                    kw_offset: kw_loc.start_offset(),
                    end_offset: end_loc.start_offset(),
                    keyword: "case",
                    style,
                },
            ));
            return;
        }

        if let Some(case_match_node) = node.as_case_match_node() {
            let kw_loc = case_match_node.case_keyword_loc();
            let end_loc = case_match_node.end_keyword_loc();
            diagnostics.extend(self.check_keyword_end(
                source,
                node,
                parse_result,
                KeywordEndContext {
                    kw_offset: kw_loc.start_offset(),
                    end_offset: end_loc.start_offset(),
                    keyword: "case",
                    style,
                },
            ));
            return;
        }

        if let Some(unless_node) = node.as_unless_node() {
            let kw_loc = unless_node.keyword_loc();
            // Only check statement-form unless (has end keyword), not modifier form
            if let Some(end_loc) = unless_node.end_keyword_loc() {
                diagnostics.extend(self.check_keyword_end(
                    source,
                    node,
                    parse_result,
                    KeywordEndContext {
                        kw_offset: kw_loc.start_offset(),
                        end_offset: end_loc.start_offset(),
                        keyword: "unless",
                        style,
                    },
                ));
            }
            return;
        }

        if let Some(sclass_node) = node.as_singleton_class_node() {
            diagnostics.extend(self.check_keyword_end(
                source,
                node,
                parse_result,
                KeywordEndContext {
                    kw_offset: sclass_node.class_keyword_loc().start_offset(),
                    end_offset: sclass_node.end_keyword_loc().start_offset(),
                    keyword: "class",
                    style,
                },
            ));
        }

        // NOTE: `begin` blocks are not checked here — that's handled by
        // Layout/BeginEndAlignment which supports variable-aligned `end`.
    }
}

impl EndAlignment {
    fn check_keyword_end(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        context: KeywordEndContext<'_>,
    ) -> Vec<Diagnostic> {
        let KeywordEndContext {
            kw_offset,
            end_offset,
            keyword,
            style,
        } = context;
        let (kw_line, _) = source.offset_to_line_col(kw_offset);
        let (end_line, end_col) = source.offset_to_line_col(end_offset);

        // Skip single-line constructs (e.g., `class Foo; end`)
        if kw_line == end_line {
            return Vec::new();
        }

        let ancestors = ancestors_for_node(parse_result, node);
        let expected_offset = match style {
            "variable" => variable_context_start_offset(source, &ancestors, node, kw_offset)
                .unwrap_or(kw_offset),
            "start_of_line" => start_of_line_alignment_offset(source, kw_offset),
            _ => kw_offset,
        };
        let expected_col = alignment_column(source, expected_offset);

        // RuboCop's effective_column subtracts 1 from all columns on line 1 when
        // there's a BOM. When the alignment target is the BOM itself (offset < 3
        // on line 1), this produces -1, which can never match an `end` on another
        // line. Replicate this by forcing a mismatch.
        let bom_forces_mismatch = {
            let bytes = source.as_bytes();
            let (exp_line, _) = source.offset_to_line_col(expected_offset);
            exp_line == 1
                && expected_offset < 3
                && bytes.len() >= 3
                && bytes[0] == 0xEF
                && bytes[1] == 0xBB
                && bytes[2] == 0xBF
        };

        if end_col != expected_col || bom_forces_mismatch {
            let msg = format!("Align `end` with `{keyword}`.");
            return vec![self.diagnostic(source, end_line, end_col, msg)];
        }

        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;
    use crate::testutil::{
        assert_cop_no_offenses_full_with_config, assert_cop_offenses_full_with_config,
    };
    use std::collections::HashMap;

    crate::cop_fixture_tests!(EndAlignment, "cops/layout/end_alignment");

    fn config_with_align_style(style: &str) -> CopConfig {
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String(style.into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn modifier_if_no_offense() {
        let source = b"x = 1 if true\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn variable_style_aligns_with_assignment() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // `x = if ...` with `end` at column 0 (start of line)
        let src = b"x = if true\n  1\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style should accept end at start of line"
        );
    }

    #[test]
    fn variable_style_no_assignment_falls_back_to_keyword() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // `super || if ...` — not an assignment, `end` should align with `if` (col 13)
        let src = b"  def foo\n    super || if true\n                 1\n             end\n  end\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style without assignment should align end with keyword: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_shovel_operator_aligns_with_line_start() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // `buf << if ...` — end aligns with line start (buf), not with `if`
        let src = b"        buf << if foo\n          bar\n        end\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style should accept end at line start for << operator: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_shovel_case_aligns_with_line_start() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // `memo << case key` — end aligns with line indent (col 4)
        let src = b"    memo << case key\n              when :a\n                1\n    end\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style should accept end at line start for << case: {:?}",
            diags
        );
    }

    #[test]
    fn bom_does_not_cause_false_positive() {
        // UTF-8 BOM + module Foo / end — correctly aligned, should not flag
        let source = b"\xEF\xBB\xBFmodule Foo\n  VERSION = '1.0'\nend\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert!(
            diags.is_empty(),
            "BOM should not cause false positive: {:?}",
            diags
        );
    }

    #[test]
    fn unless_misaligned_end_flags() {
        let source = b"unless condition\n  do_something\n  end\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert_eq!(diags.len(), 1, "should flag misaligned end for unless");
    }

    #[test]
    fn unless_aligned_end_no_offense() {
        let source = b"unless condition\n  do_something\nend\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert!(diags.is_empty(), "should not flag aligned end for unless");
    }

    #[test]
    fn case_match_misaligned_end_flags() {
        let source = b"case [1, 2]\nin [a, b]\n  a + b\n  end\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "should flag misaligned end for case/in: {:?}",
            diags
        );
    }

    #[test]
    fn singleton_class_misaligned_end_flags() {
        let source = b"class << self\n  def foo; end\n  end\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert_eq!(
            diags.len(),
            1,
            "should flag misaligned end for class << self"
        );
    }

    #[test]
    fn singleton_class_aligned_end_no_offense() {
        let source = b"class << self\n  def foo; end\nend\n";
        let diags = run_cop_full(&EndAlignment, source);
        assert!(
            diags.is_empty(),
            "should not flag aligned end for class << self"
        );
    }

    #[test]
    fn keyword_style_flags_misaligned_end_in_assignment_rhs() {
        // Exact corpus pattern: `callback_name = if block_given?` with `end` at col 6
        // The if is at col 22. In keyword style, end should align with if.
        let source = b"  def run_callback
    callback_name = if block_given?
      raise ArgumentError if method_name
      define_tmp_method(block)
    elsif method_name.is_a?(Symbol)
      define_tmp_method(method_name)
    else
      raise ArgumentError
      end
  end
";
        let diags = run_cop_full(&EndAlignment, source);
        assert!(
            diags.iter().any(|d| d.message.contains("`if`")),
            "keyword style should flag end not aligned with if in assignment RHS: {:?}",
            diags
        );
    }

    #[test]
    fn keyword_style_flags_misaligned_end_in_shovel_rhs() {
        // Exact corpus pattern: `@__body << if json` with `end` at col 6
        let source = b"  def render
    if json || plain
      @__body << if json
        json.is_a?(String) ? json : json.to_json
      else
        headers[\"content-type\"] = \"text/plain\"
        plain.to_s
      end

      @__status = 200
    end
  end
";
        let diags = run_cop_full(&EndAlignment, source);
        // Should flag `end` at col 6 not aligned with `if` at col 17
        assert!(
            diags.iter().any(|d| d.message.contains("`if`")),
            "keyword style should flag end not aligned with if in << RHS: {:?}",
            diags
        );
    }

    #[test]
    fn keyword_style_flags_misaligned_end_in_ivar_assignment_rhs() {
        // Exact corpus pattern: `@__status = if status.is_a?(Symbol)` with end at col 4
        let source = b"  def head(status)
    @__status = if status.is_a?(Symbol)
      ::Rack::Utils::SYMBOL_TO_STATUS_CODE[status]
    else
      status
    end
  end
";
        let diags = run_cop_full(&EndAlignment, source);
        // end at col 4, if at col 16 — should flag
        assert!(
            diags.iter().any(|d| d.message.contains("`if`")),
            "keyword style should flag end not aligned with if in ivar assignment RHS: {:?}",
            diags
        );
    }

    #[test]
    fn keyword_style_flags_lvar_assignment_end_at_variable_col() {
        // Pattern: `payload = if ...` where `end` is at the variable's column
        // With keyword style, end must align with `if`, not the variable
        let source = b"    payload = if auth_header
      auth_header[7..]
    elsif auth_header
      auth_header[6..]
    end
";
        let diags = run_cop_full(&EndAlignment, source);
        // `if` is at col 14, `end` is at col 4 — should flag
        assert!(
            diags.iter().any(|d| d.message.contains("`if`")),
            "keyword style: end at var col should flag when if is at different col: {:?}",
            diags
        );
    }

    #[test]
    fn keyword_style_flags_token_assignment_rhs() {
        let source = b"    token = if payload
      payload[6..]
    else
      payload
    end
";
        let diags = run_cop_full(&EndAlignment, source);
        // `if` is at col 12, `end` is at col 4 — should flag
        assert!(
            diags.iter().any(|d| d.message.contains("`if`")),
            "keyword style: token = if ... end at col 4 should flag: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_no_assignment_flags_misaligned() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("variable");
        // `super || if ...` — end NOT aligned with if (should flag)
        let src = b"  def foo\n    super || if true\n                   1\n  end\n  end\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert_eq!(
            diags.len(),
            1,
            "variable style should flag end not aligned with keyword when no assignment"
        );
    }

    #[test]
    fn start_of_line_style_allows_tabbed_alignment_fixture() {
        assert_cop_no_offenses_full_with_config(
            &EndAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/end_alignment/start_of_line_no_offense.rb"
            ),
            config_with_align_style("start_of_line"),
        );
    }

    #[test]
    fn start_of_line_style_flags_extra_tab_fixture() {
        assert_cop_offenses_full_with_config(
            &EndAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/end_alignment/start_of_line_offense.rb"
            ),
            config_with_align_style("start_of_line"),
        );
    }

    #[test]
    fn variable_style_ignores_outer_assignment_when_if_is_under_or_fixture() {
        assert_cop_no_offenses_full_with_config(
            &EndAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/end_alignment/variable_no_offense.rb"
            ),
            config_with_align_style("variable"),
        );
    }

    #[test]
    fn variable_style_aligns_with_operator_method_fixture() {
        assert_cop_offenses_full_with_config(
            &EndAlignment,
            include_bytes!("../../../tests/fixtures/cops/layout/end_alignment/variable_offense.rb"),
            config_with_align_style("variable"),
        );
    }

    #[test]
    fn variable_style_return_case_falls_back_to_keyword() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("variable");
        let src = b"def lookup(controller)\n  return case controller\n         when 'resources'\n           :resource\n         else\n           :other\n         end\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style should keep keyword alignment for return case: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_hash_value_case_falls_back_to_keyword() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("variable");
        let src = b"mapping = {\n  'type' => case kind\n            when :a\n              1\n            else\n              2\n            end\n}\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style should not align case with a hash pair: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_case_under_boolean_parent_flags_parent_aligned_end() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("variable");
        let src = b"def to_s\n  @string || case\n    when @object.nan?\n      'NaN'\n  end\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.iter().any(|d| d.message.contains("`case`")),
            "variable style should fall back to case keyword under ||: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_case_in_same_line_send_aligns_with_parent() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("variable");
        let src = b"_add_rule(case role_checks.size\n          when 0\n            raise ArgumentError, 'allow/deny should have at least 1 argument'\n          when 1 then role_checks.first\n          else\n            _either_of(role_checks)\n          end, condition)\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.iter().any(|d| d.message.contains("`case`")),
            "variable style should align same-line send(case ...) with the send: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_module_under_assignment_stays_keyword_aligned() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("variable");
        let src = b"REFINEMENT = module RefineString\n  refine String do\n  end\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.iter().any(|d| d.message.contains("`module`")),
            "variable style should keep keyword alignment for module assignment: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_module_under_send_stays_keyword_aligned() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("variable");
        let src = b"expect(module LibTest\n  extend FFI::Library\nend).to be_an_instance_of FFI::Function\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.iter().any(|d| d.message.contains("`module`")),
            "variable style should keep keyword alignment for module under send: {:?}",
            diags
        );
    }

    #[test]
    fn start_of_line_bom_flags_end() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("start_of_line");
        // BOM at file start + module/end both at col 0 — RuboCop flags because
        // effective_column is -1 for line 1 with BOM.
        let src = b"\xEF\xBB\xBFmodule Dryrun\n  VERSION = '1.0'\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.iter().any(|d| d.message.contains("`module`")),
            "start_of_line should flag end on BOM-prefixed file: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_bare_case_under_assignment_aligns_with_var() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("variable");
        // `field_class = case` (bare case) with `end` at keyword col — should flag
        let src = b"def test\n  field_class = case\n                when :a then 1\n                else 2\n                end\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.iter().any(|d| d.message.contains("`case`")),
            "variable style should flag bare case end at keyword col: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_bare_case_under_assignment_no_offense_at_var_col() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("variable");
        // `field_class = case` (bare case) with `end` at variable col — no offense
        let src = b"def test\n  field_class = case\n  when :a then 1\n  else 2\n  end\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style should accept bare case end at variable col: {:?}",
            diags
        );
    }

    #[test]
    fn variable_style_bang_bang_case_no_offense() {
        use crate::testutil::run_cop_full_with_config;

        let config = config_with_align_style("variable");
        // `!! case x` — case is receiver, not argument. Keyword alignment.
        let src = b"def test\n  !! case destroy_option\n     when Symbol, String\n       1\n     else\n       2\n     end\nend\n";
        let diags = run_cop_full_with_config(&EndAlignment, src, config);
        assert!(
            diags.is_empty(),
            "variable style should not flag !! case end at keyword col: {:?}",
            diags
        );
    }
}
