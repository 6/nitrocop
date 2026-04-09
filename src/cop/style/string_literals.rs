use ruby_prism::Visit;

use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Investigation findings:
/// - 2026-03-29: ordinary string literals inside backtick xstring
///   interpolations like `` `#{command.join(" ")}` `` must still be checked,
///   while nested dstr/dsym/regexp interpolations remain skipped.
/// - 2026-03-30: some repos enable `ConsistentQuotesInMultiline`, and Prism
///   parses plain multiline literals like `split("\n")` and `"Only in ...\n"`
///   as `StringNode`s instead of multiline `dstr`s. The corpus oracle also
///   expects the narrow `single line + trailing newline` `StringNode` form to
///   be checked even with the default config. Only skip multiline `StringNode`s
///   when they are the broader multi-line-body form RuboCop still accepts by
///   default.
/// - 2026-04-09: `double_quotes` variant had 973 FP on single-quoted strings
///   like `'\\d+'` and `'\\\\boot.ini'`. Root cause: `has_meaningful_backslash_escape`
///   paired `\\` and skipped both chars, missing that the second `\` + next
///   char (e.g. `\d`, `\b`) would become an escape in double quotes. RuboCop's
///   regex `\\[^'\\]` scans every position independently without pairing. Fixed
///   by replacing the pair-skipping loop with a simple `windows(2)` scan.
pub struct StringLiterals;

impl Cop for StringLiterals {
    fn name(&self) -> &'static str {
        "Style/StringLiterals"
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
        let enforced_style = config.get_str("EnforcedStyle", "single_quotes").to_string();
        let consistent_multiline = config.get_bool("ConsistentQuotesInMultiline", false);

        let mut visitor = StringLiteralsVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            enforced_style,
            consistent_multiline,
            in_interpolation: false,
            in_xstr: false,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct StringLiteralsVisitor<'a> {
    cop: &'a StringLiterals,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    enforced_style: String,
    consistent_multiline: bool,
    in_interpolation: bool,
    in_xstr: bool,
}

impl<'pr> Visit<'pr> for StringLiteralsVisitor<'_> {
    fn visit_embedded_statements_node(&mut self, node: &ruby_prism::EmbeddedStatementsNode<'pr>) {
        let was = self.in_interpolation;
        if !self.in_xstr {
            self.in_interpolation = true;
        }
        ruby_prism::visit_embedded_statements_node(self, node);
        self.in_interpolation = was;
    }

    fn visit_interpolated_x_string_node(
        &mut self,
        node: &ruby_prism::InterpolatedXStringNode<'pr>,
    ) {
        let was_xstr = self.in_xstr;
        self.in_xstr = true;
        ruby_prism::visit_interpolated_x_string_node(self, node);
        self.in_xstr = was_xstr;
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        let was_xstr = self.in_xstr;
        self.in_xstr = false;
        ruby_prism::visit_interpolated_string_node(self, node);
        self.in_xstr = was_xstr;
    }

    fn visit_interpolated_regular_expression_node(
        &mut self,
        node: &ruby_prism::InterpolatedRegularExpressionNode<'pr>,
    ) {
        let was_xstr = self.in_xstr;
        self.in_xstr = false;
        ruby_prism::visit_interpolated_regular_expression_node(self, node);
        self.in_xstr = was_xstr;
    }

    fn visit_interpolated_symbol_node(&mut self, node: &ruby_prism::InterpolatedSymbolNode<'pr>) {
        let was_xstr = self.in_xstr;
        self.in_xstr = false;
        ruby_prism::visit_interpolated_symbol_node(self, node);
        self.in_xstr = was_xstr;
    }

    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        let opening = match node.opening_loc() {
            Some(loc) => loc,
            None => return,
        };

        let opening_byte = opening.as_slice().first().copied().unwrap_or(0);

        // Skip %q, %Q, heredocs, ? prefix
        if matches!(opening_byte, b'%' | b'<' | b'?') {
            return;
        }

        let content = node.content_loc().as_slice();

        let is_multiline = content.contains(&b'\n');
        let ends_with_newline = content.last() == Some(&b'\n');
        let newline_count = content.iter().filter(|&&b| b == b'\n').count();
        let default_multiline_offense = ends_with_newline && newline_count == 1;

        match self.enforced_style.as_str() {
            "single_quotes" => {
                if opening_byte == b'"' {
                    // Skip if this string is inside a #{ } interpolation context —
                    // RuboCop's `inside_interpolation?` check applies to both styles.
                    if self.in_interpolation {
                        return;
                    }
                    // By default RuboCop skips plain multiline StringNode
                    // literals, but `ConsistentQuotesInMultiline: true`
                    // re-enables them.
                    if is_multiline && !self.consistent_multiline && !default_multiline_offense {
                        return;
                    }
                    // Check if single quotes can be used:
                    // - No single quotes in content
                    // - No escape sequences (no backslash in content)
                    if !util::double_quotes_required(content) {
                        let (line, column) = self.source.offset_to_line_col(opening.start_offset());
                        self.diagnostics.push(self.cop.diagnostic(self.source, line, column, "Prefer single-quoted strings when you don't need string interpolation or special symbols.".to_string()));
                    }
                }
            }
            "double_quotes" => {
                if opening_byte == b'\'' {
                    // Skip if the content contains double quotes — converting would
                    // require escaping, so the single-quoted form is preferred.
                    if content.contains(&b'"') {
                        return;
                    }
                    // Skip if the content contains a backslash followed by a
                    // character other than ' or \ — these are literal in
                    // single-quoted strings but would become escape sequences
                    // in double-quoted strings (\n, \t, \s, etc.).
                    // Backslash followed by ' or \ is OK to convert: \\ → \\
                    // and \' → '. Matches RuboCop's \\[^'\\] regex.
                    if has_meaningful_backslash_escape(content) {
                        return;
                    }
                    // Skip if content contains #@, #$, or #{ — in double quotes
                    // these would become interpolation, changing the string's meaning.
                    if content
                        .windows(2)
                        .any(|w| w == b"#{" || w == b"#@" || w == b"#$")
                    {
                        return;
                    }
                    // By default RuboCop skips plain multiline StringNode
                    // literals, but `ConsistentQuotesInMultiline: true`
                    // re-enables them.
                    if is_multiline && !self.consistent_multiline && !default_multiline_offense {
                        return;
                    }
                    // Skip if this string is inside a #{ } interpolation context —
                    // converting to double quotes would need escaping inside the
                    // enclosing double-quoted string.
                    if self.in_interpolation {
                        return;
                    }
                    let (line, column) = self.source.offset_to_line_col(opening.start_offset());
                    self.diagnostics.push(self.cop.diagnostic(self.source, line, column, "Prefer double-quoted strings unless you need single quotes to avoid extra backslashes for escaping.".to_string()));
                }
            }
            _ => {}
        }
    }
}

/// Check if a single-quoted string's raw source content contains a backslash
/// followed by a character other than `'` or `\`. In single-quoted strings,
/// `\n`, `\t`, `\s`, etc. are literal (two characters), but in double-quoted
/// strings they'd become real escape sequences. Only `\\` and `\'` are safe
/// to convert. Matches RuboCop's `\\[^'\\]` regex.
///
/// Important: this intentionally does NOT pair backslashes. RuboCop's regex
/// scans every position independently, so `'\\d+'` (source: `\\d+`) matches
/// because the second `\` followed by `d` triggers the regex, even though
/// the first `\\` is an escape pair. This prevents unsafe conversion of
/// e.g. `'\\n'` → `"\\n"` (which would change `\n` literal to a newline).
fn has_meaningful_backslash_escape(content: &[u8]) -> bool {
    content
        .windows(2)
        .any(|w| w[0] == b'\\' && w[1] != b'\'' && w[1] != b'\\')
}

/// Check if a double-quoted string's raw source content contains escape
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    crate::cop_fixture_tests!(StringLiterals, "cops/style/string_literals");

    fn consistent_multiline_config() -> CopConfig {
        CopConfig {
            options: HashMap::from([(
                "ConsistentQuotesInMultiline".into(),
                serde_yml::Value::Bool(true),
            )]),
            ..CopConfig::default()
        }
    }

    fn consistent_multiline_double_quotes_config() -> CopConfig {
        CopConfig {
            options: HashMap::from([
                (
                    "ConsistentQuotesInMultiline".into(),
                    serde_yml::Value::Bool(true),
                ),
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("double_quotes".into()),
                ),
            ]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn config_double_quotes() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // Single-quoted string should trigger with double_quotes style
        let source = b"x = 'hello'\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert!(
            !diags.is_empty(),
            "Should fire with EnforcedStyle:double_quotes on single-quoted string"
        );
        assert!(diags[0].message.contains("double-quoted"));
    }

    #[test]
    fn double_quotes_skips_inside_interpolation() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // Single-quoted string inside interpolation should NOT be flagged
        let source = b"x = \"hello #{env['KEY']}\"\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag single-quoted string inside interpolation: {:?}",
            diags
        );
    }

    #[test]
    fn double_quotes_skips_string_containing_double_quotes() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // Single-quoted string containing " should NOT be flagged
        let source = b"x = 'say \"hello\"'\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag single-quoted string with double quotes inside"
        );
    }

    #[test]
    fn double_quotes_skips_hash_brace_content() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // Single-quoted string containing #{ should NOT be flagged —
        // converting to double quotes would make it interpolation
        let source = b"x = '#{'\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag single-quoted string containing #{{: {:?}",
            diags
        );
    }

    #[test]
    fn double_quotes_skips_non_trailing_multiline_strings() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // Non-trailing multi-line single-quoted strings should still be
        // skipped by default. The trailing-newline form is covered
        // separately by the corpus regression tests.
        let source = b"x = 'hello\n  world'\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag non-trailing multi-line single-quoted string: {:?}",
            diags
        );
    }

    #[test]
    fn double_quotes_flags_string_inside_hash() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"foo(custom_attributes: { tenant_id: 'different' })\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag single-quoted string inside hash arg: {:?}",
            diags
        );
    }

    #[test]
    fn double_quotes_flags_string_after_earlier_interpolation() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // Earlier in the file there's a string with interpolation, and later a
        // single-quoted string inside a hash literal. The hash braces should NOT
        // be confused with interpolation braces.
        let source =
            b"x = \"hello #{world}\"\nfoo(custom_attributes: { tenant_id: 'different' })\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag 'different' even with earlier interpolation in file: {:?}",
            diags
        );
    }

    #[test]
    fn double_quotes_flags_escaped_backslash_in_single_quotes() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // '\\' (escaped backslash) should be flagged — can be "\\"
        let source = b"x = '\\\\'\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag '\\\\' with double_quotes style: {:?}",
            diags
        );
    }

    #[test]
    fn double_quotes_flags_escaped_single_quote() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // '\'' (escaped single quote) should be flagged — can be "'"
        let source = b"x = '\\''\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag escaped single quote with double_quotes style: {:?}",
            diags
        );
    }

    #[test]
    fn double_quotes_skips_hash_at_content() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // '#@test' should NOT be flagged — would become interpolation in double quotes
        let source = b"x = '#@test'\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag single-quoted string containing #@: {:?}",
            diags
        );
    }

    #[test]
    fn double_quotes_skips_hash_dollar_content() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // '#$test' should NOT be flagged — would become interpolation in double quotes
        let source = b"x = '#$test'\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag single-quoted string containing #$: {:?}",
            diags
        );
    }

    #[test]
    fn double_quotes_skips_backslash_n_content() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // '\n' should NOT be flagged — in double quotes \n would be a newline
        let source = b"x = '\\n'\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag single-quoted string containing \\n: {:?}",
            diags
        );
    }

    #[test]
    fn double_quotes_flags_plain_hash() {
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..CopConfig::default()
        };
        // 'blah #' should be flagged — plain # is safe in double quotes
        let source = b"x = 'blah #'\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag single-quoted string with plain # in double_quotes style: {:?}",
            diags
        );
    }

    #[test]
    fn consistent_multiline_still_skips_strings_that_require_double_quotes() {
        use crate::testutil::run_cop_full_with_config;

        let config = consistent_multiline_config();
        // The string contains \n (escape), so single quotes still can't be used
        // even when multiline checking is enabled.
        let source = b"x = \"hello\\nworld\"\n";
        let diags = run_cop_full_with_config(&StringLiterals, source, config);
        assert!(diags.is_empty());
    }

    #[test]
    fn consistent_multiline_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &StringLiterals,
            include_bytes!(
                "../../../tests/fixtures/cops/style/string_literals/consistent_multiline_offense.rb"
            ),
            consistent_multiline_config(),
        );
    }

    #[test]
    fn default_config_flags_trailing_newline_multiline_string_nodes() {
        use crate::testutil::run_cop_full;

        let source = b"s.files = `git ls-files`.split(\"\n\")\n";
        let diags = run_cop_full(&StringLiterals, source);
        assert_eq!(
            diags.len(),
            1,
            "Default config should flag trailing-newline multiline StringNodes: {:?}",
            diags
        );
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 31);
    }

    #[test]
    fn default_config_still_skips_non_trailing_multiline_string_nodes() {
        use crate::testutil::run_cop_full;

        let source = b"sql = \"SELECT * FROM foo\n       WHERE bar = baz\"\n";
        let diags = run_cop_full(&StringLiterals, source);
        assert!(
            diags.is_empty(),
            "Default config should still skip non-trailing multiline StringNodes: {:?}",
            diags
        );
    }

    #[test]
    fn default_config_still_skips_multiple_line_body_strings() {
        use crate::testutil::run_cop_full;

        let source = b"x = \"a\nb\n\"\n";
        let diags = run_cop_full(&StringLiterals, source);
        assert!(
            diags.is_empty(),
            "Default config should skip multi-line-body StringNodes: {:?}",
            diags
        );
    }

    #[test]
    fn consistent_multiline_double_quotes_flags_multiline_single_quoted_strings() {
        use crate::testutil::run_cop_full_with_config;

        let source = b"x = '\nhello\n'\n";
        let diags = run_cop_full_with_config(
            &StringLiterals,
            source,
            consistent_multiline_double_quotes_config(),
        );
        assert_eq!(
            diags.len(),
            1,
            "Consistent multiline double_quotes config should flag multiline single quotes: {:?}",
            diags
        );
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 4);
    }

    #[test]
    fn single_quotes_flags_string_inside_xstring_interpolation() {
        use crate::testutil::run_cop_full;

        let source = b"`bundle binstub vite_ruby --path #{config.root.join(\"bin\")}`\n";
        let diags = run_cop_full(&StringLiterals, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag \"bin\" inside xstring interpolation"
        );
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 52);
    }

    #[test]
    fn single_quotes_skips_nested_string_interpolation_inside_xstring() {
        use crate::testutil::run_cop_full;

        let source = b"`#{\"value: #{record.dig(\"a\", \"b\")}\"}`\n";
        let diags = run_cop_full(&StringLiterals, source);
        assert!(
            diags.is_empty(),
            "Should keep skipping strings inside nested dstr interpolation: {:?}",
            diags
        );
    }

    fn double_quotes_config() -> crate::cop::CopConfig {
        let mut opts = HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("double_quotes".to_string()),
        );
        crate::cop::CopConfig {
            options: opts,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn offense_double_quotes() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &StringLiterals,
            include_bytes!(
                "../../../tests/fixtures/cops/style/string_literals/offense.double_quotes.rb"
            ),
            double_quotes_config(),
        );
    }

    #[test]
    fn no_offense_double_quotes() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &StringLiterals,
            include_bytes!(
                "../../../tests/fixtures/cops/style/string_literals/no_offense.double_quotes.rb"
            ),
            double_quotes_config(),
        );
    }
}
