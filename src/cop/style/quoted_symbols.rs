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
/// For the `double_quotes` variant style, single-quoted symbols that START with
/// interpolation markers (`#{`, `#@`, `#$`) are not flagged because double quotes
/// would be invalid for that content (the markers would be interpreted as
/// interpolation in double quotes). This matches RuboCop's behavior for symbols
/// like `:'#{'` and `:'#$SAFE'`.
pub struct QuotedSymbols;

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

            let has_double_quote = inner.contains(&b'"');

            // For double_quotes style, check if double quotes would be invalid for this content.
            // RuboCop's invalid_double_quotes? checks for escape sequences and interpolation markers
            // in the source. For single-quoted symbols like :'#{', the source contains the literal
            // characters #{ (no backslash), but RuboCop still considers them invalid because
            // #{ would be interpreted as interpolation in double quotes.
            let has_interpolation_marker =
                inner.starts_with(b"#{") || inner.starts_with(b"#@") || inner.starts_with(b"#$");

            if style == "double_quotes" && !has_double_quote && !has_interpolation_marker {
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Prefer double-quoted symbols.".to_string(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(QuotedSymbols, "cops/style/quoted_symbols");

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
