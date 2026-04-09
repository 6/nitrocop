use crate::cop::shared::node_type::{HASH_NODE, HASH_PATTERN_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Layout/SpaceInsideHashLiteralBraces
///
/// Investigation notes (2026-04-02, FP=0, FN=9):
/// - RuboCop only skips the right-brace check when the token before `}` is a
///   single line-continuation string token, such as a double-quoted string with
///   `\\`-escaped physical newlines.
/// - Nitrocop had broadened that exemption to any multiline plain `StringNode`
///   ending immediately before `}`, which incorrectly missed real offenses for
///   multiline quoted strings, `%{}` strings, and similar closing-token shapes.
/// - Fixed by limiting the exemption to double-quoted strings whose physical
///   newlines are all backslash continuations, matching RuboCop's token stream.
///
/// Investigation notes (2026-04-09, compact FP=673, FN=13,414):
/// - `EnforcedStyle: compact` is not plain `space` style. RuboCop collapses
///   same-line successive `{ {` and `} }` token pairs in nested hashes.
/// - Nitrocop previously treated `compact` exactly like `space`, which caused
///   false positives for correctly collapsed `{{`/`}}` and false negatives for
///   spaced `{ {`/`} }` pairs that RuboCop reports as `detected`.
/// - Fixed by checking adjacent same-line brace bytes for `compact`, but only
///   collapsing a closing `}` when it is real code, not a percent-literal
///   delimiter like `%w{}` that RuboCop tokenizes differently.
pub struct SpaceInsideHashLiteralBraces;

struct BraceSpan {
    open_start: usize,
    open_end: usize,
    close_start: usize,
    has_elements: bool,
    close_follows_line_continued_double_quoted_string: bool,
}

impl SpaceInsideHashLiteralBraces {
    /// Check a hash-like node given its opening and closing brace locations.
    /// Works for both HashNode and HashPatternNode.
    #[allow(clippy::too_many_arguments)]
    fn check_hash<'pr>(
        &self,
        source: &SourceFile,
        span: &BraceSpan,
        last_element: Option<&ruby_prism::Node<'pr>>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let BraceSpan {
            open_start,
            open_end,
            close_start,
            has_elements,
            close_follows_line_continued_double_quoted_string,
        } = *span;
        let bytes = source.as_bytes();
        let empty_style = config.get_str("EnforcedStyleForEmptyBraces", "no_space");

        // Determine if the hash body is empty (only whitespace between braces)
        if !has_elements {
            let content = &bytes[open_end..close_start];
            let is_whitespace_only = content
                .iter()
                .all(|&b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'));

            if is_whitespace_only {
                if content.is_empty() {
                    // Truly empty: {}
                    if empty_style == "space" {
                        let (line, column) = source.offset_to_line_col(open_start);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside empty hash literal braces missing.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: open_end,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                    return;
                }
                // Has whitespace: { }, {  }, or multiline empty
                if empty_style == "no_space" {
                    let (line, column) = source.offset_to_line_col(open_end);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside empty hash literal braces detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: close_start,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
                return;
            }
        }

        let enforced = config.get_str("EnforcedStyle", "space");

        // RuboCop treats both spaces and tabs as valid whitespace for this check
        let space_after_open = matches!(bytes.get(open_end), Some(b' ' | b'\t'));
        let space_before_close =
            close_start > open_end && matches!(bytes.get(close_start - 1), Some(b' ' | b'\t'));

        // Check opening brace: skip if there's a line break between brace and first content,
        // or if there's a comment after the brace (also indicates line break).
        let open_content_start = scan_non_space_forward(bytes, open_end, close_start);
        let skip_open = open_content_start >= close_start
            || bytes[open_content_start] == b'\n'
            || bytes[open_content_start] == b'\r'
            || bytes[open_content_start] == b'#';
        let compact_collapse_open = !skip_open && bytes[open_content_start] == b'{';

        // Check closing brace: skip if there's a line break between last content and brace
        let close_content_end = scan_non_space_backward(bytes, open_end, close_start);
        let skip_close = close_follows_line_continued_double_quoted_string
            || close_content_end <= open_end
            || bytes[close_content_end - 1] == b'\n'
            || bytes[close_content_end - 1] == b'\r';
        let compact_collapse_close = !skip_close
            && bytes[close_content_end - 1] == b'}'
            && last_element.is_some_and(|element| {
                !ends_with_non_code_percent_literal_brace(element, close_content_end - 1)
            });

        if !skip_open {
            match enforced {
                "space" => {
                    if !space_after_open {
                        let (line, column) = source.offset_to_line_col(open_start);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside { missing.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: open_end,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
                "compact" => {
                    if compact_collapse_open {
                        if space_after_open {
                            let end = scan_space_forward(bytes, open_end);
                            let (line, column) = source.offset_to_line_col(open_end);
                            let mut diag = self.diagnostic(
                                source,
                                line,
                                column,
                                "Space inside { detected.".to_string(),
                            );
                            if let Some(ref mut corr) = corrections {
                                corr.push(crate::correction::Correction {
                                    start: open_end,
                                    end,
                                    replacement: String::new(),
                                    cop_name: self.name(),
                                    cop_index: 0,
                                });
                                diag.corrected = true;
                            }
                            diagnostics.push(diag);
                        }
                    } else if !space_after_open {
                        let (line, column) = source.offset_to_line_col(open_start);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside { missing.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: open_end,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
                "no_space" => {
                    if space_after_open {
                        let (line, column) = source.offset_to_line_col(open_end);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside { detected.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            // Find the end of spaces/tabs after the brace
                            let mut end = open_end + 1;
                            while end < close_start && matches!(bytes[end], b' ' | b'\t') {
                                end += 1;
                            }
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end,
                                replacement: String::new(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
                _ => {}
            }
        }

        if !skip_close {
            match enforced {
                "space" => {
                    if !space_before_close {
                        let (line, column) = source.offset_to_line_col(close_start);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside } missing.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: close_start,
                                end: close_start,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
                "compact" => {
                    if compact_collapse_close {
                        if space_before_close {
                            let start = scan_space_backward(bytes, close_start);
                            let (line, column) = source.offset_to_line_col(start);
                            let mut diag = self.diagnostic(
                                source,
                                line,
                                column,
                                "Space inside } detected.".to_string(),
                            );
                            if let Some(ref mut corr) = corrections {
                                corr.push(crate::correction::Correction {
                                    start,
                                    end: close_start,
                                    replacement: String::new(),
                                    cop_name: self.name(),
                                    cop_index: 0,
                                });
                                diag.corrected = true;
                            }
                            diagnostics.push(diag);
                        }
                    } else if !space_before_close {
                        let (line, column) = source.offset_to_line_col(close_start);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside } missing.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: close_start,
                                end: close_start,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
                "no_space" => {
                    if space_before_close {
                        // Find the start of spaces/tabs before the brace
                        let mut start = close_start - 1;
                        while start > open_end && matches!(bytes[start - 1], b' ' | b'\t') {
                            start -= 1;
                        }
                        let (line, column) = source.offset_to_line_col(start);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside } detected.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start,
                                end: close_start,
                                replacement: String::new(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
                _ => {}
            }
        }
    }

    fn close_follows_line_continued_double_quoted_string_value(
        source: &SourceFile,
        element: &ruby_prism::Node<'_>,
        close_start: usize,
    ) -> bool {
        let Some(assoc) = element.as_assoc_node() else {
            return false;
        };

        Self::is_line_continued_double_quoted_string_ending_at_close(
            source,
            &assoc.value(),
            close_start,
        )
    }

    fn is_line_continued_double_quoted_string_ending_at_close(
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        close_start: usize,
    ) -> bool {
        let Some(string) = node.as_string_node() else {
            return false;
        };
        let (Some(opening), Some(closing)) = (string.opening_loc(), string.closing_loc()) else {
            return false;
        };

        if opening.as_slice() != b"\"" || closing.as_slice() != b"\"" {
            return false;
        }

        if closing.start_offset() + closing.as_slice().len() != close_start {
            return false;
        }

        let content = &source.as_bytes()[opening.end_offset()..closing.start_offset()];
        Self::contains_only_line_continued_newlines(content)
    }

    fn contains_only_line_continued_newlines(content: &[u8]) -> bool {
        let mut saw_newline = false;
        let mut index = 0;

        while index < content.len() {
            match content[index] {
                b'\n' => {
                    saw_newline = true;
                    if !Self::is_line_continuation_before(content, index) {
                        return false;
                    }
                }
                b'\r' => {
                    saw_newline = true;
                    if !Self::is_line_continuation_before(content, index) {
                        return false;
                    }
                    if matches!(content.get(index + 1), Some(b'\n')) {
                        index += 1;
                    }
                }
                _ => {}
            }
            index += 1;
        }

        saw_newline
    }

    fn is_line_continuation_before(content: &[u8], newline_index: usize) -> bool {
        let mut backslashes = 0;
        let mut index = newline_index;

        while index > 0 && content[index - 1] == b'\\' {
            backslashes += 1;
            index -= 1;
        }

        backslashes % 2 == 1
    }
}

fn scan_space_forward(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    i
}

fn scan_space_backward(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
        i -= 1;
    }
    i
}

fn scan_non_space_forward(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut pos = start;
    while pos < end && matches!(bytes[pos], b' ' | b'\t') {
        pos += 1;
    }
    pos
}

fn scan_non_space_backward(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut pos = end;
    while pos > start && matches!(bytes[pos - 1], b' ' | b'\t') {
        pos -= 1;
    }
    pos
}

#[derive(Default)]
struct PercentLiteralBraceCloserVisitor {
    target_offset: usize,
    found: bool,
}

impl<'pr> Visit<'pr> for PercentLiteralBraceCloserVisitor {
    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        if self.found {
            return;
        }
        if percent_literal_array_closes_with_brace_at(node, self.target_offset) {
            self.found = true;
            return;
        }
        ruby_prism::visit_array_node(self, node);
    }

    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if self.found {
            return;
        }
        self.found = percent_literal_brace_closes_at(
            node.opening_loc(),
            node.closing_loc(),
            self.target_offset,
        );
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        if self.found {
            return;
        }
        if percent_literal_brace_closes_at(
            node.opening_loc(),
            node.closing_loc(),
            self.target_offset,
        ) {
            self.found = true;
            return;
        }
        ruby_prism::visit_interpolated_string_node(self, node);
    }

    fn visit_regular_expression_node(&mut self, node: &ruby_prism::RegularExpressionNode<'pr>) {
        if self.found {
            return;
        }
        self.found = percent_literal_brace_closes_at(
            Some(node.opening_loc()),
            Some(node.closing_loc()),
            self.target_offset,
        );
    }

    fn visit_interpolated_regular_expression_node(
        &mut self,
        node: &ruby_prism::InterpolatedRegularExpressionNode<'pr>,
    ) {
        if self.found {
            return;
        }
        if percent_literal_brace_closes_at(
            Some(node.opening_loc()),
            Some(node.closing_loc()),
            self.target_offset,
        ) {
            self.found = true;
            return;
        }
        ruby_prism::visit_interpolated_regular_expression_node(self, node);
    }

    fn visit_x_string_node(&mut self, node: &ruby_prism::XStringNode<'pr>) {
        if self.found {
            return;
        }
        self.found = percent_literal_brace_closes_at(
            Some(node.opening_loc()),
            Some(node.closing_loc()),
            self.target_offset,
        );
    }

    fn visit_interpolated_x_string_node(
        &mut self,
        node: &ruby_prism::InterpolatedXStringNode<'pr>,
    ) {
        if self.found {
            return;
        }
        if percent_literal_brace_closes_at(
            Some(node.opening_loc()),
            Some(node.closing_loc()),
            self.target_offset,
        ) {
            self.found = true;
            return;
        }
        ruby_prism::visit_interpolated_x_string_node(self, node);
    }

    fn visit_interpolated_symbol_node(&mut self, node: &ruby_prism::InterpolatedSymbolNode<'pr>) {
        if self.found {
            return;
        }
        if percent_literal_brace_closes_at(
            node.opening_loc(),
            node.closing_loc(),
            self.target_offset,
        ) {
            self.found = true;
            return;
        }
        ruby_prism::visit_interpolated_symbol_node(self, node);
    }
}

fn ends_with_non_code_percent_literal_brace(
    element: &ruby_prism::Node<'_>,
    brace_offset: usize,
) -> bool {
    let mut visitor = PercentLiteralBraceCloserVisitor {
        target_offset: brace_offset,
        found: false,
    };
    visitor.visit(element);
    visitor.found
}

fn percent_literal_array_closes_with_brace_at(
    array_node: &ruby_prism::ArrayNode<'_>,
    brace_offset: usize,
) -> bool {
    let Some(opening) = array_node.opening_loc() else {
        return false;
    };
    let Some(closing) = array_node.closing_loc() else {
        return false;
    };
    opening.as_slice().starts_with(b"%")
        && closing.as_slice() == b"}"
        && closing.start_offset() == brace_offset
}

fn percent_literal_brace_closes_at(
    opening: Option<ruby_prism::Location<'_>>,
    closing: Option<ruby_prism::Location<'_>>,
    brace_offset: usize,
) -> bool {
    let (Some(opening), Some(closing)) = (opening, closing) else {
        return false;
    };
    opening.as_slice().starts_with(b"%")
        && closing.as_slice() == b"}"
        && closing.start_offset() == brace_offset
}

impl Cop for SpaceInsideHashLiteralBraces {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideHashLiteralBraces"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[HASH_NODE, HASH_PATTERN_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        if let Some(hash) = node.as_hash_node() {
            // Note: keyword_hash_node (keyword args like `foo(a: 1)`) intentionally not
            // handled — this cop only applies to hash literals with `{ }` braces.
            let opening = hash.opening_loc();
            let closing = hash.closing_loc();
            let elements: Vec<_> = hash.elements().iter().collect();

            // Only check hash literals with { }
            if opening.as_slice() != b"{" || closing.as_slice() != b"}" {
                return;
            }

            self.check_hash(
                source,
                &BraceSpan {
                    open_start: opening.start_offset(),
                    open_end: opening.end_offset(),
                    close_start: closing.start_offset(),
                    has_elements: !elements.is_empty(),
                    close_follows_line_continued_double_quoted_string: elements.last().is_some_and(
                        |element| {
                            Self::close_follows_line_continued_double_quoted_string_value(
                                source,
                                element,
                                closing.start_offset(),
                            )
                        },
                    ),
                },
                elements.last(),
                config,
                diagnostics,
                corrections,
            );
        } else if let Some(hash_pattern) = node.as_hash_pattern_node() {
            // Hash pattern matching: `case foo; in { a: 1 }; end` or `foo in { a: 1 }`
            let opening = match hash_pattern.opening_loc() {
                Some(loc) => loc,
                None => return,
            };
            let closing = match hash_pattern.closing_loc() {
                Some(loc) => loc,
                None => return,
            };
            let elements: Vec<_> = hash_pattern.elements().iter().collect();

            // Only check patterns with { } braces (not Foo[...] syntax)
            if opening.as_slice() != b"{" || closing.as_slice() != b"}" {
                return;
            }

            self.check_hash(
                source,
                &BraceSpan {
                    open_start: opening.start_offset(),
                    open_end: opening.end_offset(),
                    close_start: closing.start_offset(),
                    has_elements: !elements.is_empty(),
                    close_follows_line_continued_double_quoted_string: elements.last().is_some_and(
                        |element| {
                            Self::close_follows_line_continued_double_quoted_string_value(
                                source,
                                element,
                                closing.start_offset(),
                            )
                        },
                    ),
                },
                elements.last(),
                config,
                diagnostics,
                corrections,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{assert_cop_no_offenses_full_with_config, run_cop_full_with_config};

    crate::cop_fixture_tests!(
        SpaceInsideHashLiteralBraces,
        "cops/layout/space_inside_hash_literal_braces"
    );
    crate::cop_autocorrect_fixture_tests!(
        SpaceInsideHashLiteralBraces,
        "cops/layout/space_inside_hash_literal_braces"
    );

    fn compact_config() -> CopConfig {
        use std::collections::HashMap;

        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("compact".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn compact_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &SpaceInsideHashLiteralBraces,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_inside_hash_literal_braces/compact_offense.rb"
            ),
            compact_config(),
        );
    }

    #[test]
    fn compact_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &SpaceInsideHashLiteralBraces,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_inside_hash_literal_braces/compact_no_offense.rb"
            ),
            compact_config(),
        );
    }

    #[test]
    fn empty_braces_space_style_flags_no_space() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForEmptyBraces".into(),
                serde_yml::Value::String("space".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"x = {}\n";
        let diags = run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, config);
        assert_eq!(
            diags.len(),
            1,
            "space style should flag empty hash without space"
        );
        assert!(diags[0].message.contains("missing"));
    }

    #[test]
    fn config_no_space() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("no_space".into()),
            )]),
            ..CopConfig::default()
        };
        // Hash with spaces should trigger with no_space style
        let source = b"x = { a: 1 }\n";
        let diags = run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, config.clone());
        assert!(
            !diags.is_empty(),
            "Should fire with EnforcedStyle:no_space on spaced hash"
        );

        // Hash without spaces should be clean with no_space style
        let source2 = b"x = {a: 1}\n";
        assert_cop_no_offenses_full_with_config(&SpaceInsideHashLiteralBraces, source2, config);
    }

    #[test]
    fn hash_pattern_matching_space_style() {
        // Default style (space) should flag missing spaces in hash patterns
        let source = b"case foo\nin {k1: 0, k2: 1}\n  nil\nend\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, CopConfig::default());
        assert_eq!(diags.len(), 2, "Should flag both braces in hash pattern");
    }

    #[test]
    fn hash_pattern_matching_no_offense() {
        let source = b"case foo\nin { k1: 0, k2: 1 }\n  nil\nend\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, CopConfig::default());
        assert_eq!(
            diags.len(),
            0,
            "Should not flag properly spaced hash pattern"
        );
    }

    #[test]
    fn multiline_empty_hash_no_space_style() {
        // Multiline empty hash should be flagged with default no_space style for empty braces
        let source = b"h = {\n}\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, CopConfig::default());
        assert_eq!(
            diags.len(),
            1,
            "Multiline empty hash with no_space should fire"
        );
    }

    #[test]
    fn empty_hash_with_multiple_spaces() {
        // {    } should be flagged with no_space style (default for empty braces)
        let source = b"h = {    }\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, CopConfig::default());
        assert_eq!(
            diags.len(),
            1,
            "Empty hash with multiple spaces should fire"
        );
    }

    #[test]
    fn compact_style_non_nested() {
        let config = compact_config();

        // Non-nested hash should be fine with spaces
        let source = b"h = { a: 1, b: 2 }\n";
        let diags = run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, config.clone());
        assert_eq!(
            diags.len(),
            0,
            "Non-nested hash in compact style should be ok"
        );

        // Hash without spaces should require them in compact style
        let source2 = b"h = {a: 1}\n";
        let diags2 = run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source2, config);
        assert_eq!(
            diags2.len(),
            2,
            "Compact should require spaces for non-nested hashes"
        );
    }

    #[test]
    fn multiline_hash_with_comment_after_brace() {
        // { # Comment should not flag the opening brace
        let source = b"h = { # Comment\n  a: 1,\n}\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, CopConfig::default());
        assert_eq!(
            diags.len(),
            0,
            "Hash with comment after brace should not flag"
        );
    }

    #[test]
    fn line_continued_double_quoted_string_last_value_does_not_flag_right_brace() {
        let source = b"response_body = { error: \"first line \\\nsecond line\"}\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, CopConfig::default());
        assert_eq!(
            diags.len(),
            0,
            "Multiline plain string values should not flag the closing brace"
        );
    }

    #[test]
    fn multiline_double_quoted_string_last_value_still_flags_right_brace() {
        let source = b"response_body = { error: \"first line\nsecond line\"}\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, CopConfig::default());
        assert_eq!(
            diags.len(),
            1,
            "Only line-continued double-quoted strings should skip the closing brace check"
        );
        assert!(diags[0].message.contains("Space inside } missing."));
    }

    #[test]
    fn multiline_call_last_value_still_flags_right_brace() {
        let source = b"response_body = { error: some_call(\n  foo)}\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, CopConfig::default());
        assert_eq!(
            diags.len(),
            1,
            "Only multiline plain strings should skip the closing brace check"
        );
        assert!(diags[0].message.contains("Space inside } missing."));
    }

    #[test]
    fn compact_style_percent_literal_value_keeps_space_before_hash_brace() {
        let source = b"translations = { month_names: %w{ Jan Feb } }\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, compact_config());
        assert_eq!(
            diags.len(),
            0,
            "Percent literal delimiters should not collapse with the enclosing hash brace"
        );
    }

    #[test]
    fn compact_style_percent_literal_value_without_space_still_offends() {
        let source = b"translations = { month_names: %w{ Jan Feb }}\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideHashLiteralBraces, source, compact_config());
        assert_eq!(
            diags.len(),
            1,
            "Compact style should still require a space before the closing hash brace here"
        );
        assert!(diags[0].message.contains("Space inside } missing."));
    }
}
