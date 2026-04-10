use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Style/SymbolArray: flags bracket arrays of symbols that could use %i.
///
/// Investigation (FP=152): The main source of false positives was missing the
/// `invalid_percent_array_context?` check from RuboCop's PercentArray mixin.
/// When a bracket symbol array is an argument to a non-parenthesized method
/// call that also has a block (e.g. `can [:admin, :read], Model do ... end`),
/// `%i[...]` would be ambiguous — Ruby cannot distinguish the `{` as a block
/// vs hash literal. RuboCop exempts these arrays and so must we.
///
/// Also added `complex_content?` check: symbols containing spaces or
/// unmatched delimiters (`[]`, `()`) cannot be represented in `%i` syntax.
///
/// ## Corpus investigation (2026-03-15)
///
/// Corpus oracle reported FP=0, FN=8,702. Match rate 62.1%.
///
/// FN=8,702: Fixed. The `in_ambiguous_block_context` flag was set for the
/// entire CallNode subtree (including the block body), but RuboCop's
/// `invalid_percent_array_context?` only checks direct arguments of the
/// non-parenthesized call. This caused every symbol array inside the block
/// body of `describe "x" do`, `it "y" do`, `context "z" do`, etc. to be
/// incorrectly suppressed — a massive miss in spec-heavy repos. Fixed by
/// scoping the flag to only the arguments subtree, not the block body.
///
/// ## Corpus investigation (2026-03-27)
///
/// Corpus oracle reported FP=3, FN=7 on 56,120 matches.
///
/// FN root causes:
/// - Prism stores both real block literals (`do ... end`, `{ ... }`) and
///   block-pass arguments (`&block`, `&(method :foo)`) in `call.block()`.
///   RuboCop's `invalid_percent_array_context?` only suppresses direct array
///   arguments of real block literals. nitrocop treated block-pass calls as
///   ambiguous too, incorrectly skipping offenses like
///   `hooks.register [:pages, :documents], :pre_render, &(method :before_render)`.
/// - The ambiguity suppression was still too broad for nested arrays. RuboCop
///   skips only the outer direct argument array, not nested symbol subarrays
///   within it. nitrocop suppressed the whole subtree, missing nested offenses
///   like `_GET_ [[:f, [:_ROOT_, :_TEMP_]], ...] do`.
/// - Invalid `%I` arrays were not implemented. RuboCop flags percent symbol
///   arrays whose contents require bracket syntax, such as `%I[#{1 + 1}]` and
///   `%I( one  two #{ 1 } )`, and includes the bracket-array replacement in the
///   message. Added the percent-array path plus RuboCop-like message building.
///
/// ## Corpus investigation (2026-04-02)
///
/// Corpus oracle reported FP=3, FN=0 on 55,881 matches.
///
/// FP root cause:
/// - Arrays containing an empty quoted symbol (`:""`) were incorrectly flagged.
///   Parser/RuboCop represents `:""` as `dsym`, so `bracketed_array_of?(:sym, node)`
///   rejects the whole array. Prism keeps it as a `SymbolNode`, so nitrocop had to
///   explicitly exclude empty symbol elements when deciding whether a bracket array
///   is a plain symbol array eligible for `%i`/`%I`.
///
/// ## Variant style investigation (2026-04-09)
///
/// Variant oracle reported 0 FP, 17,172 FN for `EnforcedStyle: brackets`.
///
/// Root cause: the `brackets` style path returned early without visiting the AST
/// at all. In brackets mode, `%i[...]` and `%I[...]` arrays should be flagged
/// with a message suggesting the equivalent bracket notation. Fixed by passing the
/// enforced style to the visitor and branching: percent arrays are flagged in
/// brackets mode (with the bracket array message), and bracket arrays are skipped.
pub struct SymbolArray;

/// Delimiter characters that cannot appear unmatched in %i arrays.
const DELIMITERS: &[char] = &['[', ']', '(', ')'];
const BARE_OPERATOR_SYMBOLS: &[&[u8]] = &[
    b"+", b"-", b"*", b"/", b"%", b"&", b"|", b"^", b"<<", b">>", b"<", b">", b"<=", b">=", b"==",
    b"!=", b"===", b"<=>", b"=~", b"!~", b"!", b"~", b"+@", b"-@", b"**", b"[]", b"[]=", b"`",
];
const SPECIAL_GLOBAL_CHARS: &[u8] = b"?!~@;,/\\=<>.*:+&`'\"0$";

fn symbol_content_is_complex(content: &str) -> bool {
    if content.contains(' ') {
        return true;
    }

    // Strip matched delimiter pairs that don't contain spaces or nested delimiters,
    // then check for remaining unmatched delimiters.
    let stripped = strip_balanced_pairs(content);
    DELIMITERS.iter().any(|d| stripped.contains(*d))
}

/// Remove balanced `[...]` and `(...)` pairs whose contents have no spaces
/// or nested delimiters. Matches RuboCop's gsub with
/// `/(\[[^\s\[\]]*\])|(\([^\s()]*\))/`.
fn strip_balanced_pairs(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' || chars[i] == '(' {
            let close = if chars[i] == '[' { ']' } else { ')' };
            // Look for matching close without spaces or nested delimiters
            if let Some(end) = find_simple_close(&chars, i + 1, close) {
                i = end + 1; // skip the matched pair
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Find a closing delimiter that contains no spaces, brackets, or parens.
fn find_simple_close(chars: &[char], start: usize, close: char) -> Option<usize> {
    for (offset, &ch) in chars[start..].iter().enumerate() {
        if ch == close {
            return Some(start + offset);
        }
        if ch.is_whitespace() || ch == '[' || ch == ']' || ch == '(' || ch == ')' {
            return None;
        }
    }
    None
}

fn is_exact_delimiter_symbol_source(node: &ruby_prism::Node<'_>) -> bool {
    let Some(sym) = node.as_symbol_node() else {
        return false;
    };
    matches!(sym.location().as_slice(), b"[" | b"]" | b"(" | b")")
}

fn source_slice(source: &SourceFile, start: usize, end: usize) -> String {
    String::from_utf8_lossy(&source.as_bytes()[start..end]).into_owned()
}

fn symbol_content_from_element(source: &SourceFile, node: &ruby_prism::Node<'_>) -> Option<String> {
    if let Some(sym) = node.as_symbol_node() {
        let content = std::str::from_utf8(sym.unescaped()).ok()?;
        return Some(content.to_string());
    }

    let interp = node.as_interpolated_symbol_node()?;
    let mut content = String::new();
    for part in interp.parts().iter() {
        if let Some(string_part) = part.as_string_node() {
            let text = std::str::from_utf8(string_part.unescaped()).ok()?;
            content.push_str(text);
        } else {
            content.push_str(&source_slice(
                source,
                part.location().start_offset(),
                part.location().end_offset(),
            ));
        }
    }
    Some(content)
}

fn array_has_complex_content(source: &SourceFile, array_node: &ruby_prism::ArrayNode<'_>) -> bool {
    for elem in array_node.elements().iter() {
        if is_exact_delimiter_symbol_source(&elem) {
            return false;
        }

        let Some(content) = symbol_content_from_element(source, &elem) else {
            return true;
        };
        if symbol_content_is_complex(&content) {
            return true;
        }
    }
    false
}

fn is_char_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic() || !ch.is_ascii()
}

fn is_char_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric() || !ch.is_ascii()
}

fn is_valid_identifier(value: &[u8]) -> bool {
    let s = match std::str::from_utf8(value) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let mut chars = s.chars();
    match chars.next() {
        Some(ch) if is_char_identifier_start(ch) => chars.all(is_char_identifier_continue),
        _ => false,
    }
}

fn is_method_name_symbol(value: &[u8]) -> bool {
    if value.is_empty() {
        return false;
    }

    let main = match value.last() {
        Some(b'!' | b'?' | b'=') => &value[..value.len() - 1],
        _ => value,
    };

    !main.is_empty() && is_valid_identifier(main)
}

fn is_instance_variable_symbol(value: &[u8]) -> bool {
    value.len() > 1 && value[0] == b'@' && is_valid_identifier(&value[1..])
}

fn is_class_variable_symbol(value: &[u8]) -> bool {
    value.len() > 2 && value.starts_with(b"@@") && is_valid_identifier(&value[2..])
}

fn is_global_variable_symbol(value: &[u8]) -> bool {
    if value.len() < 2 || value[0] != b'$' {
        return false;
    }

    if is_valid_identifier(&value[1..]) {
        return true;
    }

    if value[1].is_ascii_digit() {
        return value[2..].iter().all(|b| b.is_ascii_digit());
    }

    if value.len() == 2 && SPECIAL_GLOBAL_CHARS.contains(&value[1]) {
        return true;
    }

    value.len() == 3 && value[1] == b'-' && value[2].is_ascii_alphabetic()
}

fn can_be_unquoted_symbol(value: &[u8]) -> bool {
    is_method_name_symbol(value)
        || is_instance_variable_symbol(value)
        || is_class_variable_symbol(value)
        || is_global_variable_symbol(value)
        || BARE_OPERATOR_SYMBOLS.contains(&value)
}

fn escape_double_quoted_symbol(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\\' => escaped.push_str("\\\\"),
            b'"' => escaped.push_str("\\\""),
            b'\n' => escaped.push_str("\\n"),
            b'\r' => escaped.push_str("\\r"),
            b'\t' => escaped.push_str("\\t"),
            0x0C => escaped.push_str("\\f"),
            0x07 => escaped.push_str("\\a"),
            0x08 => escaped.push_str("\\b"),
            0x0B => escaped.push_str("\\v"),
            0x1B => escaped.push_str("\\e"),
            b'#' if i + 1 < bytes.len()
                && (bytes[i + 1] == b'{' || bytes[i + 1] == b'$' || bytes[i + 1] == b'@') =>
            {
                escaped.push_str("\\#");
            }
            _ if b < 0x20 || b == 0x7F => escaped.push_str(&format!("\\x{b:02X}")),
            _ if b > 0x7F => {
                if let Some(ch) = value[i..].chars().next() {
                    escaped.push(ch);
                    i += ch.len_utf8();
                    continue;
                }
                escaped.push_str(&format!("\\x{b:02X}"));
            }
            _ => escaped.push(b as char),
        }
        i += 1;
    }
    escaped
}

fn symbol_literal_from_value(value: &[u8]) -> String {
    match std::str::from_utf8(value) {
        Ok(value_str) if can_be_unquoted_symbol(value) => format!(":{value_str}"),
        Ok(value_str) => format!(":\"{}\"", escape_double_quoted_symbol(value_str)),
        Err(_) => {
            let mut escaped = String::new();
            for &b in value {
                if b.is_ascii_graphic() || b == b' ' {
                    escaped.push(b as char);
                } else {
                    escaped.push_str(&format!("\\x{b:02X}"));
                }
            }
            format!(":\"{escaped}\"")
        }
    }
}

fn build_bracketed_symbol_element(
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
) -> Option<String> {
    if let Some(sym) = node.as_symbol_node() {
        return Some(symbol_literal_from_value(sym.unescaped()));
    }

    let interp = node.as_interpolated_symbol_node()?;
    let mut result = String::from(":\"");
    for part in interp.parts().iter() {
        if let Some(string_part) = part.as_string_node() {
            let content = std::str::from_utf8(string_part.unescaped()).ok()?;
            result.push_str(&escape_double_quoted_symbol(content));
        } else {
            result.push_str(&source_slice(
                source,
                part.location().start_offset(),
                part.location().end_offset(),
            ));
        }
    }
    result.push('"');
    Some(result)
}

fn build_bracketed_array(source: &SourceFile, node: &ruby_prism::ArrayNode<'_>) -> Option<String> {
    let elements = node.elements();
    if elements.is_empty() {
        return Some("[]".to_string());
    }

    let opening = node.opening_loc()?;
    let closing = node.closing_loc()?;
    let element_vec: Vec<_> = elements.iter().collect();
    let leading = source_slice(
        source,
        opening.end_offset(),
        element_vec[0].location().start_offset(),
    );
    let between = if element_vec.len() >= 2 {
        source_slice(
            source,
            element_vec[0].location().end_offset(),
            element_vec[1].location().start_offset(),
        )
    } else {
        " ".to_string()
    };
    let trailing = source_slice(
        source,
        element_vec[element_vec.len() - 1].location().end_offset(),
        closing.start_offset(),
    );
    let mut converted = Vec::with_capacity(element_vec.len());
    for element in &element_vec {
        converted.push(build_bracketed_symbol_element(source, element)?);
    }

    Some(format!(
        "[{}{}{}]",
        leading,
        converted.join(&format!(",{between}")),
        trailing
    ))
}

fn build_bracket_array_message(
    source: &SourceFile,
    node: &ruby_prism::ArrayNode<'_>,
) -> Option<String> {
    let bracketed_array = build_bracketed_array(source, node)?;
    if bracketed_array.contains('\n') {
        Some("Use an array literal `[...]` for an array of symbols.".to_string())
    } else {
        Some(format!("Use `{bracketed_array}` for an array of symbols."))
    }
}

fn is_plain_symbol_element(node: &ruby_prism::Node<'_>) -> bool {
    let Some(sym) = node.as_symbol_node() else {
        return false;
    };

    // RuboCop's Parser AST represents `:""` as `dsym`, not `sym`, so arrays
    // containing it are not considered plain symbol arrays for this cop.
    !sym.unescaped().is_empty()
}

impl Cop for SymbolArray {
    fn name(&self) -> &'static str {
        "Style/SymbolArray"
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

        let mut visitor = SymbolArrayVisitor {
            cop: self,
            source,
            parse_result,
            min_size,
            enforced_style,
            diagnostics: Vec::new(),
            ambiguous_array_arg_start_offset: None,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct SymbolArrayVisitor<'a, 'src, 'pr> {
    cop: &'a SymbolArray,
    source: &'src SourceFile,
    parse_result: &'a ruby_prism::ParseResult<'pr>,
    min_size: usize,
    enforced_style: &'a str,
    diagnostics: Vec<Diagnostic>,
    /// Start offset of the direct array argument currently exempted by
    /// `invalid_percent_array_context?`.
    ambiguous_array_arg_start_offset: Option<usize>,
}

impl<'pr> SymbolArrayVisitor<'_, '_, 'pr> {
    fn check_array(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        let opening = match node.opening_loc() {
            Some(loc) => loc,
            None => return,
        };

        if opening.as_slice().starts_with(b"%i") || opening.as_slice().starts_with(b"%I") {
            // In brackets mode: all %i/%I arrays are offenses.
            // In percent mode: only flag %i/%I with complex content that needs brackets.
            let is_complex = array_has_complex_content(self.source, node);
            if self.enforced_style != "brackets" && !is_complex {
                return;
            }

            let Some(message) = build_bracket_array_message(self.source, node) else {
                return;
            };
            let (line, column) = self.source.offset_to_line_col(opening.start_offset());
            self.diagnostics
                .push(self.cop.diagnostic(self.source, line, column, message));
            return;
        }

        // Must have `[` opening (not %i or %I)
        if opening.as_slice() != b"[" {
            return;
        }

        // In brackets mode, bracket arrays are the desired style — no offense.
        if self.enforced_style == "brackets" {
            return;
        }

        let elements = node.elements();

        if elements.len() < self.min_size {
            return;
        }

        // Skip only the direct array argument in ambiguous block context.
        if self.ambiguous_array_arg_start_offset == Some(node.location().start_offset()) {
            return;
        }

        // Skip arrays containing comments — %i[] can't contain comments
        let array_start = opening.start_offset();
        let array_end = node
            .closing_loc()
            .map(|c| c.end_offset())
            .unwrap_or(array_start);
        if has_comment_in_range(self.parse_result, array_start, array_end) {
            return;
        }

        // All elements must be plain symbol nodes by RuboCop's definition.
        for elem in elements.iter() {
            if !is_plain_symbol_element(&elem) {
                return;
            }
        }

        // Skip arrays with complex content (spaces, unmatched delimiters)
        if array_has_complex_content(self.source, node) {
            return;
        }

        let (line, column) = self.source.offset_to_line_col(opening.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Use `%i` or `%I` for an array of symbols.".to_string(),
        ));
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

impl<'pr> Visit<'pr> for SymbolArrayVisitor<'_, '_, 'pr> {
    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        self.check_array(node);
        ruby_prism::visit_array_node(self, node);
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if self.is_ambiguous_block_call(node) {
            // Visit receiver normally
            if let Some(receiver) = node.receiver() {
                self.visit(&receiver);
            }
            // Visit arguments — only suppress top-level ArrayNode arguments,
            // matching RuboCop's `parent.arguments.include?(node)` check.
            // Arrays nested inside keyword hashes are NOT ambiguous.
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

    crate::cop_fixture_tests!(SymbolArray, "cops/style/symbol_array");

    #[test]
    fn config_min_size_5() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([("MinSize".into(), serde_yml::Value::Number(5.into()))]),
            ..CopConfig::default()
        };
        // 5 symbols should trigger with MinSize:5
        let source = b"x = [:a, :b, :c, :d, :e]\n";
        let diags = run_cop_full_with_config(&SymbolArray, source, config.clone());
        assert!(
            !diags.is_empty(),
            "Should fire with MinSize:5 on 5-element symbol array"
        );

        // 4 symbols should NOT trigger
        let source2 = b"x = [:a, :b, :c, :d]\n";
        let diags2 = run_cop_full_with_config(&SymbolArray, source2, config);
        assert!(
            diags2.is_empty(),
            "Should not fire on 4-element symbol array with MinSize:5"
        );
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
        let source = b"x = [:a, :b, :c]\n";
        let diags = run_cop_full_with_config(&SymbolArray, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag brackets with brackets style"
        );
    }

    fn brackets_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("brackets".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn brackets_offense() {
        use crate::testutil::assert_cop_offenses_full_with_config;
        assert_cop_offenses_full_with_config(
            &SymbolArray,
            include_bytes!("../../../tests/fixtures/cops/style/symbol_array/brackets_offense.rb"),
            brackets_config(),
        );
    }

    #[test]
    fn brackets_no_offense() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        assert_cop_no_offenses_full_with_config(
            &SymbolArray,
            include_bytes!(
                "../../../tests/fixtures/cops/style/symbol_array/brackets_no_offense.rb"
            ),
            brackets_config(),
        );
    }
}
