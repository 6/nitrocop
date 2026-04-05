use crate::cop::shared::node_type::{INTERPOLATED_STRING_NODE, STRING_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Variant fix: `percent_q` style uses RuboCop regex `/^%[^\w]/` where `\w`
/// includes `[a-zA-Z0-9_]`. The original Rust code only checked
/// `is_ascii_alphabetic()`, missing digits and underscores. The corpus FP
/// was `%_..._` (underscore delimiter) in `natalie-lang/natalie` —
/// RuboCop's `\w` matches `_`, so `%_` is not flagged. Fixed by adding
/// `is_ascii_alphanumeric()` and `!= b'_'` checks.
pub struct BarePercentLiterals;

impl Cop for BarePercentLiterals {
    fn name(&self) -> &'static str {
        "Style/BarePercentLiterals"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[INTERPOLATED_STRING_NODE, STRING_NODE]
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
        let enforced_style = config.get_str("EnforcedStyle", "bare_percent");

        // Check both StringNode and InterpolatedStringNode
        let (opening_loc, node_loc) = if let Some(s) = node.as_string_node() {
            (s.opening_loc(), s.location())
        } else if let Some(s) = node.as_interpolated_string_node() {
            (s.opening_loc(), s.location())
        } else {
            return;
        };

        let opening = match opening_loc {
            Some(loc) => loc,
            None => return,
        };

        let opening_bytes = opening.as_slice();

        match enforced_style {
            "bare_percent" => {
                // Flag %Q usage
                if opening_bytes.starts_with(b"%Q") {
                    let (line, column) = source.offset_to_line_col(node_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `%` instead of `%Q`.".to_string(),
                    ));
                }
            }
            "percent_q" => {
                // Flag bare % usage (not %q, %Q, etc. - just % followed by a non-alpha)
                if opening_bytes.starts_with(b"%")
                    && !opening_bytes.starts_with(b"%Q")
                    && !opening_bytes.starts_with(b"%q")
                    && opening_bytes.len() >= 2
                    && !opening_bytes[1].is_ascii_alphanumeric()
                    && opening_bytes[1] != b'_'
                {
                    let (line, column) = source.offset_to_line_col(node_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `%Q` instead of `%`.".to_string(),
                    ));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(BarePercentLiterals, "cops/style/bare_percent_literals");

    #[test]
    fn percent_q_style_skips_underscore_delimiter() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("percent_q".into()),
            )]),
            ..CopConfig::default()
        };
        // %_ uses underscore as delimiter — RuboCop treats _ as \w, so
        // /^%[^\w]/ does NOT match. We must not flag it.
        let source = b"x = %_hello world_\n";
        let diags = run_cop_full_with_config(&BarePercentLiterals, source, config);
        assert!(
            diags.is_empty(),
            "Should not flag %_ with percent_q style: {:?}",
            diags
        );
    }

    #[test]
    fn percent_q_style_flags_bare_percent_parens() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("percent_q".into()),
            )]),
            ..CopConfig::default()
        };
        // %(hello) should still be flagged with percent_q style
        let source = b"x = %(hello world)\n";
        let diags = run_cop_full_with_config(&BarePercentLiterals, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag %() with percent_q style: {:?}",
            diags
        );
    }
}
