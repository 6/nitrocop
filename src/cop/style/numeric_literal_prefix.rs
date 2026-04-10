use crate::cop::shared::node_type::INTEGER_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Checks for octal, hex, binary, and decimal literals using uppercase prefixes
/// and corrects them to lowercase prefix or no prefix (in case of decimals).
///
/// ## Investigation notes (2026-03-18)
///
/// **FP root cause:** `0_30` (underscore-separated decimal starting with 0) was
/// incorrectly flagged as an octal literal. The old code stripped all underscores
/// before matching, turning `0_30` into `030` which matched the octal pattern.
/// RuboCop's regexes (`/^0O?[0-7]+$/`) match the original source without stripping
/// underscores, so `0_30` correctly does NOT match because `_` is not in `[0-7]`.
/// Fix: match the original source text without stripping underscores.
///
/// **FN root cause:** Negative integer literals like `-0O1` and `-01234` were missed.
/// Prism includes the `-` sign in the `IntegerNode` location for negative literals,
/// so `src_str` started with `-` and none of the `starts_with("0...")` checks matched.
/// RuboCop's `integer_part` helper strips leading `+`/`-` before checking.
/// Fix: strip leading sign before prefix matching, adjust column offset by 1.
///
/// **FP root cause (complex/rational suffixes):** `042i` and `042r` are complex and
/// rational number literals, not plain octals. Prism parses these as `ImaginaryNode`
/// or `RationalNode` wrapping an `IntegerNode`. The AST walker visits the inner
/// `IntegerNode`, which has source text `042` (without suffix), matching the octal
/// pattern. RuboCop's `on_int` only fires for standalone `:int` nodes (Parser gem uses
/// distinct `:complex`/`:rational` types). Fix: check the byte after the `IntegerNode`
/// location — if it's `i` or `r`, skip the node.
///
/// ## Investigation notes (2026-04-07) — variant: EnforcedOctalStyle: zero_only
///
/// **FP root cause (zero_only variant):** In `zero_only` mode, the cop was flagging
/// octal literals with underscores like `0o100_666` and `0o100_644`. RuboCop's
/// `OCTAL_ZERO_ONLY_REGEX` is `/^0[Oo][0-7]+$/` which does NOT match literals
/// containing underscores (since `_` is not in `[0-7]`). The old code only checked
/// `literal.starts_with("0o")` or `literal.starts_with("0O")` without verifying
/// that the suffix contained only octal digits. Fix: added `&& literal[2..].bytes().all(|b|
/// b.is_ascii_digit() && b < b'8')` to match RuboCop's regex behavior.
pub struct NumericLiteralPrefix;

impl Cop for NumericLiteralPrefix {
    fn name(&self) -> &'static str {
        "Style/NumericLiteralPrefix"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[INTEGER_NODE]
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
        let int_node = match node.as_integer_node() {
            Some(i) => i,
            None => return,
        };

        let loc = int_node.location();
        let src = loc.as_slice();

        // Skip integer literals that are part of complex (042i) or rational (042r) literals.
        // Prism visits the IntegerNode child inside ImaginaryNode/RationalNode, but RuboCop's
        // on_int callback only fires for standalone int nodes (Parser gem uses :complex/:rational
        // node types, not :int). Check the byte after the IntegerNode location.
        let source_bytes = source.as_bytes();
        let end = loc.start_offset() + src.len();
        if end < source_bytes.len() {
            let next_byte = source_bytes[end];
            if next_byte == b'i' || next_byte == b'r' {
                return;
            }
        }
        let src_str = match std::str::from_utf8(src) {
            Ok(s) => s,
            Err(_) => return,
        };

        // Strip leading +/- sign, like RuboCop's integer_part helper.
        // Do NOT strip underscores — RuboCop's regexes match the original source
        // including underscores, so `0_30` correctly does NOT match octal patterns.
        let (literal, sign_offset) = if src_str.starts_with('+') || src_str.starts_with('-') {
            (&src_str[1..], 1usize)
        } else {
            (src_str, 0usize)
        };

        let enforced_octal_style = config.get_str("EnforcedOctalStyle", "zero_with_o");

        let (line, column) = source.offset_to_line_col(loc.start_offset());
        // Offset the column past the sign character so the diagnostic points at
        // the numeric literal, not the sign.
        let col = column + sign_offset;

        // Check uppercase hex prefix: 0X...
        if literal.starts_with("0X") {
            diagnostics.push(self.diagnostic(
                source,
                line,
                col,
                "Use 0x for hexadecimal literals.".to_string(),
            ));
        }

        // Check uppercase binary prefix: 0B...
        if literal.starts_with("0B") {
            diagnostics.push(self.diagnostic(
                source,
                line,
                col,
                "Use 0b for binary literals.".to_string(),
            ));
        }

        // Check decimal prefix: 0d... or 0D...
        if literal.starts_with("0d") || literal.starts_with("0D") {
            diagnostics.push(self.diagnostic(
                source,
                line,
                col,
                "Do not use prefixes for decimal literals.".to_string(),
            ));
        }

        // Octal handling
        if enforced_octal_style == "zero_with_o" {
            // Bad: 0O... (uppercase)
            if literal.starts_with("0O") {
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    col,
                    "Use 0o for octal literals.".to_string(),
                ));
            }
            // Bad: plain 0... without 'o' (e.g., 01234)
            // Must be octal: starts with 0, followed by digits 0-7 only, not 0x/0b/0d/0o
            // Underscores in the source mean it's a decimal with visual separators (e.g. 0_30),
            // not an octal literal.
            if literal.len() > 1
                && literal.starts_with('0')
                && !literal.starts_with("0x")
                && !literal.starts_with("0b")
                && !literal.starts_with("0o")
                && literal[1..].bytes().all(|b| b.is_ascii_digit() && b < b'8')
            {
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    col,
                    "Use 0o for octal literals.".to_string(),
                ));
            }
        } else if enforced_octal_style == "zero_only" {
            // Bad: 0o... or 0O... ONLY if the suffix after 0o/0O contains
            // only octal digits (matching RuboCop's OCTAL_ZERO_ONLY_REGEX).
            // Octal literals with underscores like 0o100_666 should NOT be flagged.
            if (literal.starts_with("0o") || literal.starts_with("0O"))
                && literal[2..].bytes().all(|b| b.is_ascii_digit() && b < b'8')
            {
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    col,
                    "Use 0 for octal literals.".to_string(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(NumericLiteralPrefix, "cops/style/numeric_literal_prefix");

    fn zero_only_config() -> CopConfig {
        let mut opts = std::collections::HashMap::new();
        opts.insert(
            "EnforcedOctalStyle".to_string(),
            serde_yml::Value::String("zero_only".to_string()),
        );
        CopConfig {
            options: opts,
            ..CopConfig::default()
        }
    }

    #[test]
    fn zero_only_no_offense_octal_with_underscore() {
        // 0o100_666 should NOT be flagged in zero_only mode - RuboCop's
        // OCTAL_ZERO_ONLY_REGEX (/^0[Oo][0-7]+$/) does not match because
        // underscore is not an octal digit
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &NumericLiteralPrefix,
            include_bytes!(
                "../../../tests/fixtures/cops/style/numeric_literal_prefix/no_offense.zero_only.rb"
            ),
            zero_only_config(),
        );
    }
}
