use crate::cop::shared::node_type::SYMBOL_NODE;
use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Matches the corpus oracle's empty-symbol edge case for hash labels.
///
/// Standalone empty quoted symbols like `:""` remain accepted, but empty-string
/// hash-label keys like `"":` are still checked for quote style. This fixes the
/// remaining FN cluster without broadening handling for multiline or escaped
/// quoted symbols.
///
/// For the `double_quotes` variant style, single-quoted symbols stay accepted when
/// converting them to double quotes would change semantics by activating
/// interpolation (`:'#{'`, `:'#$SAFE'`) or real double-quote escape sequences
/// (`:'\000'`). Ordinary escaped single quotes and doubled backslashes remain
/// offenses (`:'o\'clock'`, `:'foo\\bar'`), matching RuboCop's narrower
/// `invalid_double_quotes?` check.
pub struct QuotedSymbols;

fn has_double_quotes_only_escape(inner: &[u8]) -> bool {
    for i in 0..inner.len().saturating_sub(1) {
        if inner[i] == b'\\'
            && (i == 0 || inner[i - 1] != b'\\')
            && matches!(
                inner[i + 1],
                b'a' | b'A'
                    | b'b'
                    | b'c'
                    | b'd'
                    | b'e'
                    | b'f'
                    | b'k'
                    | b'M'
                    | b'n'
                    | b'p'
                    | b'r'
                    | b's'
                    | b'S'
                    | b't'
                    | b'u'
                    | b'U'
                    | b'x'
                    | b'z'
                    | b'Z'
                    | b'0'..=b'7'
            )
        {
            return true;
        }
    }

    false
}

impl Cop for QuotedSymbols {
    fn name(&self) -> &'static str {
        "Style/QuotedSymbols"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[SYMBOL_NODE]
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
        let style = config.get_str("EnforcedStyle", "same_as_string_literals");

        let sym = match node.as_symbol_node() {
            Some(s) => s,
            None => return,
        };

        let loc = sym.location();
        let src_bytes = loc.as_slice();

        // Determine if this is a hash-key symbol (e.g. "invest": or 'invest':)
        // vs a standalone symbol (e.g. :"foo" or :'foo')
        let is_hash_key_double = src_bytes.starts_with(b"\"") && src_bytes.ends_with(b"\":");
        let is_hash_key_single = src_bytes.starts_with(b"'") && src_bytes.ends_with(b"':");
        let is_standalone_double = src_bytes.starts_with(b":\"");
        let is_standalone_single = src_bytes.starts_with(b":'");

        let is_double_quoted = is_hash_key_double || is_standalone_double;
        let is_single_quoted = is_hash_key_single || is_standalone_single;

        if is_double_quoted {
            // Unterminated symbol literal (parse error) — bail out.
            if src_bytes.len() < 3 {
                return;
            }
            // Extract inner content (between the quotes)
            let inner = if is_hash_key_double {
                &src_bytes[1..src_bytes.len().saturating_sub(2)] // strip leading " and trailing ":
            } else {
                &src_bytes[2..src_bytes.len().saturating_sub(1)] // strip leading :" and trailing "
            };
            if inner.is_empty() && !is_hash_key_double {
                return;
            }
            if inner.contains(&b'\n') || inner.contains(&b'\r') {
                return;
            }

            let has_interpolation = inner
                .windows(2)
                .any(|w| w == b"#{" || w == b"#@" || w == b"#$");

            if has_interpolation {
                return; // Double quotes needed
            }

            let prefer_single = match style {
                "single_quotes" => true,
                "same_as_string_literals" => {
                    let sl_style = config.get_str("StringLiteralsEnforcedStyle", "single_quotes");
                    sl_style != "double_quotes"
                }
                "double_quotes" => false,
                _ => true,
            };

            let string_literal_src = if is_hash_key_double {
                src_bytes
            } else {
                &src_bytes[1..]
            };

            if prefer_single && !util::double_quotes_required(string_literal_src) {
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Prefer single-quoted symbols when you don't need string interpolation or special symbols.".to_string(),
                ));
            }
        } else if is_single_quoted {
            // Unterminated symbol literal (parse error) — bail out.
            if src_bytes.len() < 3 {
                return;
            }
            let inner = if is_hash_key_single {
                &src_bytes[1..src_bytes.len().saturating_sub(2)] // strip leading ' and trailing ':
            } else {
                &src_bytes[2..src_bytes.len().saturating_sub(1)] // strip leading :' and trailing '
            };
            if inner.is_empty() && !is_hash_key_single {
                return;
            }
            if inner.contains(&b'\n') || inner.contains(&b'\r') {
                return;
            }

            let can_stay_single_quoted = inner.contains(&b'"')
                || inner
                    .windows(2)
                    .any(|w| w == b"#{" || w == b"#@" || w == b"#$")
                || has_double_quotes_only_escape(inner);

            if style == "double_quotes" && !can_stay_single_quoted {
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Prefer double-quoted symbols unless you need single quotes to avoid extra backslashes for escaping.".to_string(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(QuotedSymbols, "cops/style/quoted_symbols");

    fn double_quotes_config() -> crate::cop::CopConfig {
        use std::collections::HashMap;
        crate::cop::CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("double_quotes".into()),
            )]),
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn double_quotes_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &QuotedSymbols,
            include_bytes!(
                "../../../tests/fixtures/cops/style/quoted_symbols/double_quotes_offense.rb"
            ),
            double_quotes_config(),
        );
    }

    #[test]
    fn double_quotes_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &QuotedSymbols,
            include_bytes!(
                "../../../tests/fixtures/cops/style/quoted_symbols/double_quotes_no_offense.rb"
            ),
            double_quotes_config(),
        );
    }

    /// Regression test: unterminated symbol literals (parse errors) must not panic.
    /// Found by fuzz_all_cops with input `:'`.
    #[test]
    fn unterminated_symbol_no_panic() {
        use crate::cop::Cop;
        use crate::cop::walker::BatchedCopWalker;
        use ruby_prism::Visit;

        for input in [":'", ":\"", ":'hello", ":\"hello"] {
            let source = crate::parse::source::SourceFile::from_string(
                std::path::PathBuf::from("fuzz.rb"),
                input.to_string(),
            );
            let parse_result = crate::parse::parse_source(source.as_bytes());
            let config = crate::cop::CopConfig::default();
            let cop = QuotedSymbols;
            let ast_cops: Vec<(&dyn Cop, &crate::cop::CopConfig)> = vec![(&cop, &config)];
            let mut walker = BatchedCopWalker::new(ast_cops, &source, &parse_result);
            walker.visit(&parse_result.node());
        }
    }
}
