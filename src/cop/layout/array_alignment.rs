use crate::cop::shared::util::begins_its_line;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Equivalent to RuboCop's `source_line =~ /\S/`. Handles both spaces and tabs,
/// each counted as 1 character. Unlike `shared::util::indentation_of` which only
/// counts spaces, this matches RuboCop's behavior for tab-indented code.
fn first_non_whitespace_column(line: &[u8]) -> usize {
    line.iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count()
}

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
///
/// ## `with_fixed_indentation` variant fixes (2026-04-08)
///
/// **FN root cause (3,209 FN):** `check_element_alignment` used `skip(1)` and
/// initialized `last_checked_line = first_line`, so the first element was never
/// checked. With `with_first_element` this is correct (first element defines the
/// expected column), but with `with_fixed_indentation` the expected column is
/// `indentation_of(bracket_line) + indent_width`, so the first element on its own
/// line can be misaligned. RuboCop's `each_bad_alignment` starts with
/// `prev_line = -1` and checks ALL elements. Fixed by introducing `anchor_line`
/// (the bracket/parent/keyword line) and iterating all elements.
///
/// **FP root cause (rescue, 2 FP):** `check_rescue_exceptions` used the first
/// exception's line for indentation, but RuboCop uses `node.parent.loc.line`
/// (the rescue keyword's line). Differs when `rescue \` continuation puts the
/// first exception on a new line. Fixed by using `rescue_node.keyword_loc()`.
///
/// **FP root cause (bracketless arrays, ~6 FP):** For non-bracketed arrays with
/// `with_fixed_indentation`, RuboCop uses `node.parent.loc.line`. We used the
/// first element's line. Fixed by tracking parent node lines via
/// `visit_branch_node_enter`/`leave` and using the parent's line.
///
/// **FP/FN root cause (tabs):** `indentation_of()` only counts spaces, returning 0
/// for tab-indented lines. RuboCop uses `/\S/ =~ line` which counts both tabs and
/// spaces as 1 character each. Fixed by using `first_non_whitespace_column()`.
///
/// **Message:** `with_fixed_indentation` uses a different message than the default:
/// "Use one level of indentation for elements following the first line of a
/// multi-line array." vs the default "Align the elements of an array literal if
/// they span more than one line."
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
            node_line_stack: Vec::new(),
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
    /// Stack of node start lines, maintained via visit_branch_node_enter/leave.
    /// When visiting an array node, the parent's start line is the second-to-last
    /// element (the last element is the array itself, pushed by
    /// visit_branch_node_enter before visit_array_node runs).
    node_line_stack: Vec<usize>,
}

impl<'pr> Visit<'pr> for AlignmentVisitor<'_> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        let (line, _) = self
            .source
            .offset_to_line_col(node.location().start_offset());
        self.node_line_stack.push(line);
    }

    fn visit_branch_node_leave(&mut self) {
        self.node_line_stack.pop();
    }

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
            // For bracketless arrays (implicit), RuboCop's `target_method_lineno`
            // uses `node.parent.loc.line`. We get the parent's line from the
            // node_line_stack: the second-to-last entry is the parent (the last
            // entry is this array node itself, pushed by visit_branch_node_enter).
            let parent_line = if self.node_line_stack.len() >= 2 {
                Some(self.node_line_stack[self.node_line_stack.len() - 2])
            } else {
                None
            };
            self.cop.check_array(
                self.source,
                node,
                self.config,
                self.diagnostics,
                parent_line,
            );
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
        parent_line: Option<usize>,
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

        let is_fixed = style == "with_fixed_indentation";

        let expected_col = match style {
            "with_fixed_indentation" => {
                if is_bracketed {
                    let open_loc = array_node.opening_loc().unwrap();
                    let (open_line, _) = source.offset_to_line_col(open_loc.start_offset());
                    let open_line_bytes = source.lines().nth(open_line - 1).unwrap_or(b"");
                    first_non_whitespace_column(open_line_bytes) + indent_width
                } else {
                    // For bracketless arrays (implicit from trailing comma or method
                    // args), RuboCop uses node.parent.loc.line to find the base
                    // indentation. We use the parent_line from the visitor's node
                    // stack. This is critical for cases like:
                    //   config.cache_store =
                    //     :memory_store,
                    //     { size: 128 }
                    // where the parent (CallNode at line of `config.cache_store =`)
                    // has indentation 2, giving expected_col = 4, NOT the first
                    // element's line indentation (4) + indent_width = 6.
                    let base_line = parent_line.unwrap_or(first_line);
                    let base_line_bytes = source.lines().nth(base_line - 1).unwrap_or(b"");
                    first_non_whitespace_column(base_line_bytes) + indent_width
                }
            }
            _ => first_col, // "with_first_element" (default)
        };

        // RuboCop's each_bad_alignment starts with prev_line = -1 and checks ALL
        // elements. For `with_first_element`, the first element trivially matches
        // (expected_col == first_col). For `with_fixed_indentation`, the first
        // element on its own line may be misaligned vs the bracket/parent line.
        // We use anchor_line as the "already checked" line: for default style it's
        // first_line (so first element is skipped); for fixed_indentation with
        // brackets it's the `[` line (so first element on a new line is checked).
        let anchor_line = if is_fixed && is_bracketed {
            let open_loc = array_node.opening_loc().unwrap();
            let (open_line, _) = source.offset_to_line_col(open_loc.start_offset());
            open_line
        } else if is_fixed {
            parent_line.unwrap_or(first_line)
        } else {
            first_line
        };

        self.check_element_alignment(
            source,
            &elements,
            anchor_line,
            expected_col,
            is_fixed,
            diagnostics,
        );
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

        let is_fixed = style == "with_fixed_indentation";

        let expected_col = match style {
            "with_fixed_indentation" => {
                // RuboCop treats rescue exception lists as arrays whose parent is
                // the resbody. For `with_fixed_indentation`, the base line is the
                // parent's (resbody's) line — which is the rescue keyword's line.
                // This is critical for line-continued rescues like:
                //   rescue \
                //     FooError,
                //     BarError => e
                // where the rescue keyword is on a different line than the first
                // exception. We must use the keyword's line (indentation 8), not
                // the first exception's line (indentation 10).
                let keyword_loc = rescue_node.keyword_loc();
                let (keyword_line, _) = source.offset_to_line_col(keyword_loc.start_offset());
                let keyword_line_bytes = source.lines().nth(keyword_line - 1).unwrap_or(b"");
                first_non_whitespace_column(keyword_line_bytes) + indent_width
            }
            _ => first_col, // "with_first_element" (default)
        };

        // For rescue, the anchor line is the rescue keyword's line for
        // with_fixed_indentation (so the first exception on its own line
        // is checked), or first_line for the default style.
        let anchor_line = if is_fixed {
            let keyword_loc = rescue_node.keyword_loc();
            let (keyword_line, _) = source.offset_to_line_col(keyword_loc.start_offset());
            keyword_line
        } else {
            first_line
        };

        self.check_element_alignment(
            source,
            &exceptions,
            anchor_line,
            expected_col,
            is_fixed,
            diagnostics,
        );
    }

    fn check_element_alignment(
        &self,
        source: &SourceFile,
        elements: &ruby_prism::NodeList<'_>,
        anchor_line: usize,
        expected_col: usize,
        is_fixed_indentation: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        // RuboCop's each_bad_alignment starts with prev_line = -1 and iterates
        // ALL elements. We use anchor_line as the "already seen" line:
        // - For with_first_element: anchor_line = first_line, so the first
        //   element (same line) is skipped.
        // - For with_fixed_indentation: anchor_line = bracket/parent/keyword line,
        //   so the first element on a new line IS checked.
        let mut last_checked_line = anchor_line;

        let message = if is_fixed_indentation {
            "Use one level of indentation for elements following the first line of a multi-line array."
        } else {
            "Align the elements of an array literal if they span more than one line."
        };

        for elem in elements.iter() {
            let start_offset = elem.location().start_offset();
            let (elem_line, elem_col) = source.offset_to_line_col(start_offset);
            // Only check the first element on each new line; subsequent elements
            // on the same line are just comma-separated and not alignment targets.
            if elem_line == last_checked_line {
                // Update last_checked_line even when skipping, matching RuboCop's
                // `prev_line = current.loc.line` at the end of each iteration.
                continue;
            }
            last_checked_line = elem_line;
            // Skip elements that are not the first non-whitespace token on their line.
            // E.g. in `}, {` the `{` follows a `}` and should not be checked.
            if !begins_its_line(source, start_offset) {
                continue;
            }
            if elem_col != expected_col {
                diagnostics.push(self.diagnostic(source, elem_line, elem_col, message.to_string()));
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

    #[test]
    fn with_fixed_indentation_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ArrayAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/array_alignment/with_fixed_indentation_offense.rb"
            ),
            {
                let mut options = std::collections::HashMap::new();
                options.insert(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("with_fixed_indentation".into()),
                );
                CopConfig {
                    options,
                    ..CopConfig::default()
                }
            },
        );
    }

    #[test]
    fn with_fixed_indentation_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ArrayAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/array_alignment/with_fixed_indentation_no_offense.rb"
            ),
            {
                let mut options = std::collections::HashMap::new();
                options.insert(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("with_fixed_indentation".into()),
                );
                CopConfig {
                    options,
                    ..CopConfig::default()
                }
            },
        );
    }

    #[test]
    fn with_fixed_indentation_bracketless_array_parent_line() {
        // Regression test: bracketless arrays (implicit from trailing comma in
        // assignment) should use the parent statement's line indentation, not the
        // first element's line. Matches RuboCop's target_method_lineno behavior.
        //
        // Source:
        //   config.cache_store =    <- parent line, indent 2
        //     :memory_store,        <- first element, indent 4
        //     { size: 128 }         <- second element, indent 4 (should be OK)
        //
        // Expected: parent indent (2) + indent_width (2) = 4. Both elements at col 4.
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("with_fixed_indentation".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"  config.cache_store =\n    :memory_store,\n    { size: 128 }\n";
        let diags = run_cop_full_with_config(&ArrayAlignment, src, config);
        assert!(
            diags.is_empty(),
            "bracketless array with elements at parent_indent+2 should not be flagged, got {:?}",
            diags
        );
    }

    #[test]
    fn with_fixed_indentation_rescue_line_continuation() {
        // Regression test: rescue with line continuation should use the rescue
        // keyword's line indentation, not the first exception's line indentation.
        //
        // Source:
        //         rescue \              <- keyword line, indent 8
        //           FooError,           <- first exception, indent 10
        //           BarError => e       <- second exception, indent 10 (should be OK)
        //
        // Expected: keyword indent (8) + indent_width (2) = 10. Both exceptions at col 10.
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("with_fixed_indentation".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"begin\n  run_command\nrescue \\\n  FooError,\n  BarError => e\n  handle\nend\n";
        let diags = run_cop_full_with_config(&ArrayAlignment, src, config);
        assert!(
            diags.is_empty(),
            "rescue with line continuation should not flag aligned exceptions, got {:?}",
            diags
        );
    }
}
