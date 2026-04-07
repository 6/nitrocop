use crate::cop::shared::util::begins_its_line;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Layout/ArrayAlignment checks alignment of multi-line array literal elements
/// and rescue exception lists.
///
/// ## Investigation findings (2026-03-14)
///
/// **FP root cause (original):** Prism wraps multi-assignment RHS values (`a, b = 1, 2`)
/// in an implicit `ArrayNode` with no `opening_loc`. Fixed by skipping arrays inside
/// `MultiWriteNode` parents, matching RuboCop's `return if node.parent&.masgn_type?`.
///
/// **FP root cause (2026-03-17):** Bracketed arrays inside multi-assignments
/// (e.g., `a, b = [x, y]`) were still being checked. RuboCop skips ALL arrays
/// whose parent is a masgn, regardless of brackets. Fixed by moving array checking
/// to a visitor that tracks parent context via `in_multi_write` flag.
///
/// **FN root cause (2026-03-23):** When multi-write has multiple RHS values
/// (e.g., `a, b = [x, y], z`), Prism wraps them in an implicit ArrayNode.
/// The `in_multi_write` flag propagated into ALL children, skipping the nested
/// bracketed `[x, y]` array. But RuboCop's `node.parent&.masgn_type?` check
/// only skips arrays whose immediate parent is the masgn, not arrays nested
/// inside the implicit RHS wrapper. Fixed by resetting `in_multi_write` before
/// visiting array children.
///
/// **FN root cause (original):** RuboCop treats rescue exception lists as arrays
/// for alignment. In Prism these are `RescueNode` with `exceptions()` list.
/// Fixed by adding rescue node handling.
///
/// **FN root cause (2026-03-17):** Trailing commas in assignments create implicit
/// arrays (e.g., `x = val,\n  next_line`). Prism wraps these in `ArrayNode` with
/// no `opening_loc`, same as multi-assignment RHS. The blanket skip of bracketless
/// arrays missed these. Fixed by only skipping arrays inside `MultiWriteNode`,
/// not all bracketless arrays.
///
/// **FN root cause (2026-03-18):** Arrays inside if/else bodies within
/// multi-assignments (`a, b = if cond; [x, y]; end`) were skipped because
/// `in_multi_write` propagated through the entire MultiWriteNode subtree.
/// RuboCop only skips arrays whose immediate parent is the masgn. Fixed by
/// manually visiting MultiWriteNode children and only setting `in_multi_write`
/// when the direct value is an ArrayNode, not for non-array values like IfNode.
pub struct ArrayAlignment;

impl Cop for ArrayAlignment {
    fn name(&self) -> &'static str {
        "Layout/ArrayAlignment"
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
        let mut visitor = AlignmentVisitor {
            cop: self,
            source,
            config,
            diagnostics,
            in_multi_write: false,
        };
        visitor.visit(&parse_result.node());
    }
}

struct AlignmentVisitor<'a> {
    cop: &'a ArrayAlignment,
    source: &'a SourceFile,
    config: &'a CopConfig,
    diagnostics: &'a mut Vec<Diagnostic>,
    in_multi_write: bool,
}

impl<'pr> Visit<'pr> for AlignmentVisitor<'_> {
    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        // RuboCop: `return if node.parent&.masgn_type?` — only skips arrays
        // whose IMMEDIATE parent is the multi-write node. We replicate the
        // default visitor manually and set `in_multi_write` only when the
        // direct `value()` is itself an ArrayNode (implicit or bracketed).
        // If the value is something else (e.g., IfNode), we visit it normally
        // so arrays nested deeper (inside if/else bodies) are still checked.
        for child in &node.lefts() {
            self.visit(&child);
        }
        if let Some(rest) = node.rest() {
            self.visit(&rest);
        }
        for child in &node.rights() {
            self.visit(&child);
        }
        let value = node.value();
        if value.as_array_node().is_some() {
            // Direct array child of multi-write — skip alignment check
            let prev = self.in_multi_write;
            self.in_multi_write = true;
            self.visit(&value);
            self.in_multi_write = prev;
        } else {
            // Non-array value (e.g., IfNode, MethodCall) — visit normally
            self.visit(&value);
        }
    }

    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        // RuboCop: `return if node.parent&.masgn_type?`
        // Skip only the direct array child of MultiWriteNode (implicit or bracketed).
        // Nested arrays within the multi-write value (e.g., `a, b = [x, y], z`
        // where `[x, y]` is inside the implicit RHS array) ARE checked, since
        // their parent is the implicit array, not the masgn itself.
        if !self.in_multi_write {
            self.cop
                .check_array(self.source, node, self.config, self.diagnostics);
        }
        // Reset in_multi_write before visiting children — only the direct
        // array child of MultiWriteNode is skipped, not nested arrays.
        let prev = self.in_multi_write;
        self.in_multi_write = false;
        ruby_prism::visit_array_node(self, node);
        self.in_multi_write = prev;
    }

    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        self.cop
            .check_rescue_exceptions(self.source, node, self.config, self.diagnostics);
        ruby_prism::visit_rescue_node(self, node);
    }
}

impl ArrayAlignment {
    fn check_array(
        &self,
        source: &SourceFile,
        array_node: &ruby_prism::ArrayNode<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let style = config.get_str("EnforcedStyle", "with_first_element");
        let indent_width = config.get_usize("IndentationWidth", 2);
        let is_bracketed = array_node.opening_loc().is_some();

        let elements = array_node.elements();
        if elements.len() < 2 {
            return;
        }

        let first = match elements.iter().next() {
            Some(e) => e,
            None => return,
        };
        let (first_line, first_col) = source.offset_to_line_col(first.location().start_offset());

        let expected_col = match style {
            "with_fixed_indentation" => {
                if is_bracketed {
                    let open_loc = array_node.opening_loc().unwrap();
                    let (open_line, _) = source.offset_to_line_col(open_loc.start_offset());
                    let open_line_bytes = source.lines().nth(open_line - 1).unwrap_or(b"");
                    crate::cop::shared::util::indentation_of(open_line_bytes) + indent_width
                } else {
                    // For bracketless arrays (trailing comma), use first element's
                    // line indentation + indent_width
                    let first_line_bytes = source.lines().nth(first_line - 1).unwrap_or(b"");
                    crate::cop::shared::util::indentation_of(first_line_bytes) + indent_width
                }
            }
            _ => first_col, // "with_first_element" (default)
        };

        self.check_element_alignment(source, &elements, first_line, expected_col, diagnostics);
    }

    fn check_rescue_exceptions(
        &self,
        source: &SourceFile,
        rescue_node: &ruby_prism::RescueNode<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let style = config.get_str("EnforcedStyle", "with_first_element");
        let indent_width = config.get_usize("IndentationWidth", 2);
        let exceptions = rescue_node.exceptions();
        if exceptions.len() < 2 {
            return;
        }

        let first = match exceptions.iter().next() {
            Some(e) => e,
            None => return,
        };
        let (first_line, first_col) = source.offset_to_line_col(first.location().start_offset());

        let expected_col = match style {
            "with_fixed_indentation" => {
                // Use the rescue keyword line's indentation + indent_width
                let rescue_line_bytes = source.lines().nth(first_line - 1).unwrap_or(b"");
                crate::cop::shared::util::indentation_of(rescue_line_bytes) + indent_width
            }
            _ => first_col, // "with_first_element" (default)
        };

        self.check_element_alignment(source, &exceptions, first_line, expected_col, diagnostics);
    }

    fn check_element_alignment(
        &self,
        source: &SourceFile,
        elements: &ruby_prism::NodeList<'_>,
        first_line: usize,
        expected_col: usize,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let mut last_checked_line = first_line;

        for elem in elements.iter().skip(1) {
            let start_offset = elem.location().start_offset();
            let (elem_line, elem_col) = source.offset_to_line_col(start_offset);
            // Only check the first element on each new line; subsequent elements
            // on the same line are just comma-separated and not alignment targets.
            if elem_line == last_checked_line {
                continue;
            }
            last_checked_line = elem_line;
            // Skip elements that are not the first non-whitespace token on their line.
            // E.g. in `}, {` the `{` follows a `}` and should not be checked.
            if !begins_its_line(source, start_offset) {
                continue;
            }
            if elem_col != expected_col {
                diagnostics.push(
                    self.diagnostic(
                        source,
                        elem_line,
                        elem_col,
                        "Align the elements of an array literal if they span more than one line."
                            .to_string(),
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(ArrayAlignment, "cops/layout/array_alignment");

    #[test]
    fn rescue_exception_list_misaligned() {
        // rescue exceptions not aligned with first exception
        let source =
            b"begin\n  foo\nrescue ArgumentError,\n  RuntimeError,\n  TypeError => e\n  bar\nend\n";
        let diags = run_cop_full(&ArrayAlignment, source);
        assert_eq!(
            diags.len(),
            2,
            "should flag both misaligned rescue exceptions"
        );
    }

    #[test]
    fn rescue_exception_list_aligned() {
        // rescue exceptions aligned with first exception — no offense
        let source = b"begin\n  foo\nrescue ArgumentError,\n       RuntimeError,\n       TypeError => e\n  bar\nend\n";
        let diags = run_cop_full(&ArrayAlignment, source);
        assert!(
            diags.is_empty(),
            "aligned rescue exceptions should not be flagged"
        );
    }

    #[test]
    fn rescue_single_exception_no_offense() {
        let source = b"begin\n  foo\nrescue ArgumentError => e\n  bar\nend\n";
        let diags = run_cop_full(&ArrayAlignment, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn single_line_array_no_offense() {
        let source = b"x = [1, 2, 3]\n";
        let diags = run_cop_full(&ArrayAlignment, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn with_fixed_indentation_style() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("with_fixed_indentation".into()),
            )]),
            ..CopConfig::default()
        };
        // Elements at fixed indentation (2 spaces) should be accepted
        let src = b"x = [\n  1,\n  2\n]\n";
        let diags = run_cop_full_with_config(&ArrayAlignment, src, config.clone());
        assert!(
            diags.is_empty(),
            "with_fixed_indentation should accept 2-space indent"
        );

        // Elements aligned with first element at column 4 should be flagged
        let src2 = b"x = [1,\n     2]\n";
        let diags2 = run_cop_full_with_config(&ArrayAlignment, src2, config);
        assert_eq!(
            diags2.len(),
            1,
            "with_fixed_indentation should flag first-element alignment"
        );
    }
}
