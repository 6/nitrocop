use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Default WordRegex pattern matching RuboCop's default:
/// `/\A(?:\p{Word}|\p{Word}-\p{Word}|\n|\t)+\z/`
/// Translated to Rust regex syntax: \A → ^, \z → $, \p{Word} → \w
const DEFAULT_WORD_REGEX: &str = r"^(?:\w|\w-\w|\n|\t)+$";

/// Style/WordArray: flags bracket arrays of word-like strings that could use %w.
///
/// ## Investigation findings
///
/// **FP fix 1:** Missing `within_matrix_of_complex_content?` check from RuboCop.
/// When a bracket array is nested inside a parent array (a "matrix") where ALL
/// elements are arrays, and at least ONE sibling subarray has complex content
/// (strings with spaces, non-word characters, or invalid encoding), RuboCop
/// exempts the entire matrix.
///
/// **FP fix 2:** Missing `invalid_percent_array_context?` check from RuboCop's
/// PercentArray mixin. When a bracket word array is an argument to a
/// non-parenthesized method call that also has a block (e.g.,
/// `describe_pattern "LOG", ['legacy', 'ecs-v1'] do ... end`), `%w()`
/// would be ambiguous — Ruby cannot distinguish `{` as a block vs hash
/// literal. RuboCop exempts these arrays and so must we. (FP=107 fixed)
///
/// **FN fix:** The `is_matrix_of_complex_content` function was incorrectly
/// treating non-string elements (integers, symbols) as "complex content".
/// RuboCop's `complex_content?` method skips non-string elements (`next unless
/// s.str_content`). This caused parent arrays with mixed-type subarrays
/// (e.g., `[["foo", "bar", 0], ["baz", "qux"]]`) to be wrongly classified as
/// complex-content matrices, suppressing pure-string subarrays. (FN=314 fixed)
///
/// **FN fix 2 (percent-to-brackets):** When enforced style is `percent`
/// (default), `%w`/`%W` arrays whose elements contain spaces (via backslash
/// escaping, e.g. `%w(Cucumber\ features features)`) should be flagged for
/// conversion to bracket syntax `[...]`. This matches RuboCop's
/// `invalid_percent_array_contents?` check. Single-line arrays get a message
/// with the explicit bracket form; multi-line arrays use the generic
/// `Use an array literal [...]` message.
///
/// **FN fix 3:** The matrix suppression state was leaking to all descendants of
/// a complex matrix. RuboCop only skips arrays whose direct parent is the
/// matrix returned by `within_matrix_of_complex_content?`. Nested word arrays
/// inside those skipped rows, such as `["Europe", ["Denmark", ...]]` and
/// `["বাংলা", "bn", ["bn-BD", "বাংলাদেশ"]]`, must still be checked.
///
/// **FN fix 4:** Prism stores both real block literals (`do ... end`, `{}`)
/// and block-pass arguments (`&cb`) in `call.block()`. RuboCop's
/// `invalid_percent_array_context?` only suppresses a bracket array when it is
/// a direct argument to a non-parenthesized call with a real block literal.
/// nitrocop treated block-pass calls as ambiguous too, and it also suppressed
/// nested arrays inside the direct argument array. That missed offenses like
/// `d.handle ['foobar', 'barfoo'], &cb`, which RuboCop flags.
///
/// **Remaining FN:** Primarily `brackets` style enforcement direction
/// (flagging ALL `%w[...]` arrays for conversion to brackets), which is not
/// yet implemented.
pub struct WordArray;

/// Extract a Ruby regexp pattern from a string like `/pattern/flags`.
/// Returns the inner pattern without delimiters and flags.
fn extract_word_regex(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.starts_with('/') && s.len() > 1 {
        if let Some(end) = s[1..].rfind('/') {
            return Some(&s[1..end + 1]);
        }
    }
    None
}

/// Translate Ruby regex syntax to Rust regex syntax.
fn translate_ruby_regex(pattern: &str) -> String {
    pattern
        .replace(r"\A", "^")
        .replace(r"\z", "$")
        .replace(r"\p{Word}", r"\w")
}

/// Build a compiled regex from the WordRegex config value.
/// Falls back to the default pattern if the config value is empty or unparseable.
fn build_word_regex(config_value: &str) -> Option<regex::Regex> {
    if config_value.is_empty() {
        return regex::Regex::new(DEFAULT_WORD_REGEX).ok();
    }
    let raw_pattern = if let Some(inner) = extract_word_regex(config_value) {
        inner
    } else {
        config_value
    };
    let translated = translate_ruby_regex(raw_pattern);
    regex::Regex::new(&translated).ok()
}

/// Check if an array node has complex content (any string element that doesn't
/// match the word regex, contains spaces, is empty, or has invalid encoding).
fn array_has_complex_content(
    array_node: &ruby_prism::ArrayNode<'_>,
    word_re: &Option<regex::Regex>,
) -> bool {
    for elem in array_node.elements().iter() {
        let string_node = match elem.as_string_node() {
            Some(s) => s,
            None => return true, // non-string element = complex
        };
        if string_node.opening_loc().is_none() {
            return true;
        }
        let unescaped_bytes = string_node.unescaped();
        if unescaped_bytes.is_empty() {
            return true;
        }
        if unescaped_bytes.contains(&b' ') {
            return true;
        }
        let content_str = match std::str::from_utf8(unescaped_bytes) {
            Ok(s) => s,
            Err(_) => return true,
        };
        if let Some(re) = word_re {
            if !re.is_match(content_str) {
                return true;
            }
        }
    }
    false
}

/// Check if a subarray has complex string content, matching RuboCop's
/// `complex_content?` semantics. Non-string elements are SKIPPED (not treated
/// as complex). Only string elements with spaces, empty content, invalid
/// encoding, or non-word characters count as complex.
fn subarray_has_complex_string_content(
    array_node: &ruby_prism::ArrayNode<'_>,
    word_re: &Option<regex::Regex>,
) -> bool {
    for elem in array_node.elements().iter() {
        let string_node = match elem.as_string_node() {
            Some(s) => s,
            None => continue, // skip non-string elements (RuboCop: `next unless s.str_content`)
        };
        if string_node.opening_loc().is_none() {
            continue;
        }
        let unescaped_bytes = string_node.unescaped();
        // Empty strings and strings with spaces are complex
        if unescaped_bytes.is_empty() || unescaped_bytes.contains(&b' ') {
            return true;
        }
        let content_str = match std::str::from_utf8(unescaped_bytes) {
            Ok(s) => s,
            Err(_) => return true,
        };
        if let Some(re) = word_re {
            if !re.is_match(content_str) {
                return true;
            }
        }
    }
    false
}

/// Check if a parent array is a "matrix of complex content": all elements are
/// arrays, and at least one has complex string content. Matches RuboCop's
/// `matrix_of_complex_content?` method.
fn is_matrix_of_complex_content(
    array_node: &ruby_prism::ArrayNode<'_>,
    word_re: &Option<regex::Regex>,
) -> bool {
    let elements = array_node.elements();
    if elements.is_empty() {
        return false;
    }
    let mut any_complex = false;
    for elem in elements.iter() {
        let sub = match elem.as_array_node() {
            Some(a) => a,
            None => return false, // not all elements are arrays
        };
        if !any_complex && subarray_has_complex_string_content(&sub, word_re) {
            any_complex = true;
        }
    }
    any_complex
}

impl Cop for WordArray {
    fn name(&self) -> &'static str {
        "Style/WordArray"
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
        let min_size = config.get_usize("MinSize", 2);
        let enforced_style = config.get_str("EnforcedStyle", "percent");
        let word_regex_str = config.get_str("WordRegex", "");

        if enforced_style == "brackets" {
            let mut visitor = BracketsStyleVisitor {
                cop: self,
                source,
                diagnostics: Vec::new(),
            };
            visitor.visit(&parse_result.node());
            diagnostics.extend(visitor.diagnostics);
            return;
        }

        let word_re = build_word_regex(word_regex_str);

        let mut visitor = WordArrayVisitor {
            cop: self,
            source,
            parse_result,
            min_size,
            word_re,
            parent_is_complex_matrix: false,
            ambiguous_array_arg_start_offset: None,
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct WordArrayVisitor<'a, 'src, 'pr> {
    cop: &'a WordArray,
    source: &'src SourceFile,
    parse_result: &'a ruby_prism::ParseResult<'pr>,
    min_size: usize,
    word_re: Option<regex::Regex>,
    /// True when the direct parent array is a complex matrix that suppresses
    /// only its immediate child arrays, matching RuboCop's
    /// `within_matrix_of_complex_content?`.
    parent_is_complex_matrix: bool,
    /// Start offset of the direct array argument to a non-parenthesized call
    /// with a real block literal. Only that array is ambiguous for `%w`.
    ambiguous_array_arg_start_offset: Option<usize>,
    diagnostics: Vec<Diagnostic>,
}

impl<'pr> WordArrayVisitor<'_, '_, 'pr> {
    fn check_array(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        // Must have `[` opening (not %w or %W)
        let opening = match node.opening_loc() {
            Some(loc) => loc,
            None => return,
        };

        if opening.as_slice() != b"[" {
            return;
        }

        let elements = node.elements();

        if elements.len() < self.min_size {
            return;
        }

        // Skip if inside a matrix of complex content
        if self.parent_is_complex_matrix {
            return;
        }

        // Skip only the direct array argument in ambiguous block context.
        if self.ambiguous_array_arg_start_offset == Some(node.location().start_offset()) {
            return;
        }

        // Skip arrays that contain comments
        let array_start = opening.start_offset();
        let array_end = node
            .closing_loc()
            .map(|c| c.end_offset())
            .unwrap_or(array_start);
        if has_comment_in_range(self.parse_result, array_start, array_end) {
            return;
        }

        // All elements must be simple string nodes with word-like content
        if array_has_complex_content(node, &self.word_re) {
            return;
        }

        let (line, column) = self.source.offset_to_line_col(opening.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Use `%w` or `%W` for an array of words.".to_string(),
        ));
    }

    /// Check a `%w` or `%W` array for invalid percent contents (spaces).
    /// When enforced style is `percent`, percent arrays with spaces should
    /// use bracket syntax instead.
    fn check_percent_word_array(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        let opening = match node.opening_loc() {
            Some(loc) => loc,
            None => return,
        };

        let opening_bytes = opening.as_slice();
        if opening_bytes.len() < 2
            || opening_bytes[0] != b'%'
            || (opening_bytes[1] != b'w' && opening_bytes[1] != b'W')
        {
            return;
        }

        if !has_invalid_percent_word_contents(node) {
            return;
        }

        let closing_offset = node.closing_loc().map(|c| c.start_offset());
        let message = build_percent_offense_message(
            node,
            self.source,
            opening.start_offset(),
            closing_offset,
        );

        let (line, column) = self.source.offset_to_line_col(opening.start_offset());
        self.diagnostics
            .push(self.cop.diagnostic(self.source, line, column, message));
    }

    /// Check if a call node represents an ambiguous block context:
    /// non-parenthesized method call with a block.
    fn is_ambiguous_block_call(&self, call: &ruby_prism::CallNode<'pr>) -> bool {
        // Must have a real block. BlockArgumentNode (`&block`, `&(method :foo)`)
        // is not ambiguous for percent arrays.
        if call
            .block()
            .is_none_or(|block| block.as_block_node().is_none())
        {
            return false;
        }
        // Must have arguments
        if call.arguments().is_none() {
            return false;
        }
        // Must NOT be parenthesized
        call.opening_loc().is_none()
    }
}

impl<'pr> Visit<'pr> for WordArrayVisitor<'_, '_, 'pr> {
    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        self.check_array(node);
        self.check_percent_word_array(node);

        // Only direct children of a complex matrix are suppressed. Nested arrays
        // inside those child rows must still be checked.
        let prev = self.parent_is_complex_matrix;
        self.parent_is_complex_matrix = is_matrix_of_complex_content(node, &self.word_re);
        ruby_prism::visit_array_node(self, node);
        self.parent_is_complex_matrix = prev;
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if self.is_ambiguous_block_call(node) {
            // Visit receiver normally
            if let Some(receiver) = node.receiver() {
                self.visit(&receiver);
            }
            // Visit arguments — only suppress top-level ArrayNode arguments,
            // matching RuboCop's `parent.arguments.include?(node)` check.
            // Arrays nested inside that direct argument are still checked.
            if let Some(args) = node.arguments() {
                let prev = self.ambiguous_array_arg_start_offset;
                for arg in args.arguments().iter() {
                    if let Some(array) = arg.as_array_node() {
                        self.ambiguous_array_arg_start_offset =
                            Some(array.location().start_offset());
                        self.visit(&arg);
                        self.ambiguous_array_arg_start_offset = prev;
                    } else {
                        self.visit(&arg);
                    }
                }
            }
            // Visit block normally — arrays inside block body are NOT ambiguous
            if let Some(block) = node.block() {
                self.visit(&block);
            }
        } else {
            ruby_prism::visit_call_node(self, node);
        }
    }
}

/// Visitor for `EnforcedStyle: brackets` — flags `%w`/`%W` arrays for
/// conversion to bracket syntax.
struct BracketsStyleVisitor<'a, 'src> {
    cop: &'a WordArray,
    source: &'src SourceFile,
    diagnostics: Vec<Diagnostic>,
}

impl<'pr> BracketsStyleVisitor<'_, '_> {
    fn check_percent_word_array(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        let opening = match node.opening_loc() {
            Some(loc) => loc,
            None => return,
        };

        let opening_bytes = opening.as_slice();
        if opening_bytes.len() < 2
            || opening_bytes[0] != b'%'
            || (opening_bytes[1] != b'w' && opening_bytes[1] != b'W')
        {
            return;
        }

        let message = self.build_brackets_message(node, &opening);

        let (line, column) = self.source.offset_to_line_col(opening.start_offset());
        self.diagnostics
            .push(self.cop.diagnostic(self.source, line, column, message));
    }

    fn build_brackets_message(
        &self,
        node: &ruby_prism::ArrayNode<'pr>,
        opening: &ruby_prism::Location<'_>,
    ) -> String {
        let elements = node.elements();

        if elements.is_empty() {
            return "Use `[]` for an array of words.".to_string();
        }

        // Check if single-line
        let start_line = self.source.offset_to_line_col(opening.start_offset()).0;
        let end_line = node
            .closing_loc()
            .map(|c| self.source.offset_to_line_col(c.start_offset()).0)
            .unwrap_or(start_line);

        if start_line != end_line {
            return "Use an array literal `[...]` for an array of words.".to_string();
        }

        // Single-line: build explicit bracket form from element contents
        let mut words = Vec::new();
        for elem in elements.iter() {
            let string_node = match elem.as_string_node() {
                Some(s) => s,
                None => {
                    // Non-string element (e.g., interpolated %W) — fall back
                    return "Use an array literal `[...]` for an array of words.".to_string();
                }
            };
            let unescaped = string_node.unescaped();
            let content = match std::str::from_utf8(unescaped) {
                Ok(s) => s,
                Err(_) => {
                    return "Use an array literal `[...]` for an array of words.".to_string();
                }
            };
            if content.contains('\'') {
                words.push(format!("\"{}\"", content));
            } else {
                words.push(format!("'{}'", content));
            }
        }

        // Build bracket form preserving original whitespace
        let bracket_array =
            build_bracket_array_with_whitespace(node, &words, self.source.as_bytes());
        if bracket_array.contains('\n') {
            "Use an array literal `[...]` for an array of words.".to_string()
        } else {
            format!("Use `{}` for an array of words.", bracket_array)
        }
    }
}

impl<'pr> Visit<'pr> for BracketsStyleVisitor<'_, '_> {
    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        self.check_percent_word_array(node);
        ruby_prism::visit_array_node(self, node);
    }
}

/// Build a bracket array representation preserving the whitespace from the
/// original `%w`/`%W` source. Mirrors RuboCop's
/// `build_bracketed_array_with_appropriate_whitespace`.
fn build_bracket_array_with_whitespace(
    node: &ruby_prism::ArrayNode<'_>,
    quoted_elements: &[String],
    source_bytes: &[u8],
) -> String {
    let elements = node.elements();
    if elements.is_empty() {
        return "[]".to_string();
    }

    let opening_end = node.opening_loc().unwrap().end_offset();
    let closing_start = node
        .closing_loc()
        .map(|c| c.start_offset())
        .unwrap_or(opening_end);

    let first_start = elements.iter().next().unwrap().location().start_offset();
    let last_end = elements.iter().last().unwrap().location().end_offset();

    let leading = std::str::from_utf8(&source_bytes[opening_end..first_start]).unwrap_or("");
    let trailing = std::str::from_utf8(&source_bytes[last_end..closing_start]).unwrap_or("");

    let between = if elements.len() >= 2 {
        let first_end = elements.iter().next().unwrap().location().end_offset();
        let second_start = elements.iter().nth(1).unwrap().location().start_offset();
        std::str::from_utf8(&source_bytes[first_end..second_start]).unwrap_or(" ")
    } else {
        " "
    };

    let mut result = String::from("[");
    result.push_str(leading);
    for (i, elem) in quoted_elements.iter().enumerate() {
        if i > 0 {
            result.push(',');
            result.push_str(between);
        }
        result.push_str(elem);
    }
    result.push_str(trailing);
    result.push(']');
    result
}

/// Check if a `%w` or `%W` array has invalid contents for percent syntax:
/// any string element that contains a space or has invalid encoding.
/// Matches RuboCop's `invalid_percent_array_contents?` override in WordArray.
fn has_invalid_percent_word_contents(node: &ruby_prism::ArrayNode<'_>) -> bool {
    for elem in node.elements().iter() {
        let string_node = match elem.as_string_node() {
            Some(s) => s,
            None => continue, // skip non-string elements (interpolated, etc.)
        };
        let unescaped = string_node.unescaped();
        if unescaped.contains(&b' ') {
            return true;
        }
        if std::str::from_utf8(unescaped).is_err() {
            return true;
        }
    }
    false
}

/// Build the bracket array representation for the offense message.
/// Returns the full message string.
fn build_percent_offense_message(
    node: &ruby_prism::ArrayNode<'_>,
    source: &SourceFile,
    opening_offset: usize,
    closing_offset: Option<usize>,
) -> String {
    let start_line = source.offset_to_line_col(opening_offset).0;
    let end_line = closing_offset
        .map(|o| source.offset_to_line_col(o).0)
        .unwrap_or(start_line);

    if start_line != end_line {
        return "Use an array literal `[...]` for an array of words.".to_string();
    }

    // Single-line: build the bracket representation
    let mut words = Vec::new();
    for elem in node.elements().iter() {
        if let Some(string_node) = elem.as_string_node() {
            let unescaped = string_node.unescaped();
            let content = String::from_utf8_lossy(unescaped);
            if content.contains('\'') {
                words.push(format!("\"{}\"", content));
            } else {
                words.push(format!("'{}'", content));
            }
        } else {
            return "Use an array literal `[...]` for an array of words.".to_string();
        }
    }
    format!("Use `[{}]` for an array of words.", words.join(", "))
}

/// Check if there are any comments within a byte offset range.
fn has_comment_in_range(
    parse_result: &ruby_prism::ParseResult<'_>,
    start: usize,
    end: usize,
) -> bool {
    for comment in parse_result.comments() {
        let comment_start = comment.location().start_offset();
        if comment_start >= start && comment_start < end {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(WordArray, "cops/style/word_array");

    #[test]
    fn config_min_size_5() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([("MinSize".into(), serde_yml::Value::Number(5.into()))]),
            ..CopConfig::default()
        };
        // 5 elements should trigger with MinSize:5
        let source = b"x = ['a', 'b', 'c', 'd', 'e']\n";
        let diags = run_cop_full_with_config(&WordArray, source, config.clone());
        assert!(
            !diags.is_empty(),
            "Should fire with MinSize:5 on 5-element word array"
        );

        // 4 elements should NOT trigger
        let source2 = b"x = ['a', 'b', 'c', 'd']\n";
        let diags2 = run_cop_full_with_config(&WordArray, source2, config);
        assert!(
            diags2.is_empty(),
            "Should not fire on 4-element word array with MinSize:5"
        );
    }

    #[test]
    fn default_word_regex_rejects_hyphens_only() {
        let re = build_word_regex("").unwrap();
        assert!(!re.is_match("-"), "single hyphen should not match");
        assert!(!re.is_match("----"), "multiple hyphens should not match");
        assert!(re.is_match("foo"), "simple word should match");
        assert!(re.is_match("foo-bar"), "hyphenated word should match");
        assert!(re.is_match("one\n"), "word with newline should match");
        assert!(!re.is_match(" "), "space should not match");
        assert!(!re.is_match(""), "empty should not match");
    }

    #[test]
    fn brackets_style_allows_bracket_arrays() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("brackets".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"x = ['a', 'b', 'c']\n";
        let diags = run_cop_full_with_config(&WordArray, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag brackets with brackets style"
        );
    }

    #[test]
    fn matrix_with_mixed_types_does_not_suppress_pure_string_subarrays() {
        use crate::testutil::run_cop_full;

        // Parent array where some subarrays mix strings and integers.
        // RuboCop's complex_content? skips non-strings, so the matrix
        // is NOT considered "complex content". Pure-string subarrays
        // like ["foo", "bar"] should still be flagged.
        let source = b"x = [[\"foo\", \"bar\", 0], [\"baz\", \"qux\"]]\n";
        let diags = run_cop_full(&WordArray, source);
        assert!(
            !diags.is_empty(),
            "Should flag pure-string subarrays in a matrix with mixed-type siblings"
        );
    }

    #[test]
    fn all_word_matrix_flags_subarrays() {
        use crate::testutil::run_cop_full;

        // Matrix where all subarrays are simple word arrays.
        // RuboCop flags each subarray since there's no complex content.
        let source = b"[['Architecture', 'view_architectures'], ['Audit', 'view_audit_logs']]\n";
        let diags = run_cop_full(&WordArray, source);
        assert_eq!(
            diags.len(),
            2,
            "Should flag both subarrays in an all-word matrix"
        );
    }

    fn brackets_config() -> CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("brackets".to_string()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    #[test]
    fn brackets_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &WordArray,
            include_bytes!("../../../tests/fixtures/cops/style/word_array/brackets_offense.rb"),
            brackets_config(),
        );
    }

    #[test]
    fn brackets_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &WordArray,
            include_bytes!("../../../tests/fixtures/cops/style/word_array/brackets_no_offense.rb"),
            brackets_config(),
        );
    }

    #[test]
    fn ambiguous_block_context_skips_only_direct_array_arg() {
        use crate::testutil::run_cop_full;

        let block_pass = b"d.handle ['foobar', 'barfoo'], &cb\n";
        let block_pass_diags = run_cop_full(&WordArray, block_pass);
        assert_eq!(
            block_pass_diags.len(),
            1,
            "Block-pass calls are not ambiguous for `%w` and should still be flagged"
        );

        let nested = b"foo [['bar', 'baz']] do\nend\n";
        let nested_diags = run_cop_full(&WordArray, nested);
        assert_eq!(
            nested_diags.len(),
            1,
            "Only the direct array arg is ambiguous; nested word arrays must still be checked"
        );
    }
}
