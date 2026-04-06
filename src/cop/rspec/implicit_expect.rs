use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Checks that a consistent implicit expectation style is used.
///
/// With `EnforcedStyle: should`, the cop flags `is_expected.to`, `is_expected.to_not`,
/// and `is_expected.not_to` patterns. The message must correctly reflect which matcher
/// is used: `.to` suggests `should`, while `.to_not` and `.not_to` suggest `should_not`.
/// The fix examines the source text after `is_expected` to determine the matcher and
/// constructs the appropriate message.
pub struct ImplicitExpect;

impl Cop for ImplicitExpect {
    fn name(&self) -> &'static str {
        "RSpec/ImplicitExpect"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_DEFAULT_INCLUDE
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
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
        // Config: EnforcedStyle — "is_expected" (default) or "should"
        let enforced_style = config.get_str("EnforcedStyle", "is_expected");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        if call.receiver().is_some() {
            return;
        }

        let method_name = call.name().as_slice();

        if enforced_style == "should" {
            // "should" style: flag `is_expected`
            if method_name == b"is_expected" {
                let loc = call.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());

                // Determine the matcher used after is_expected to construct the correct message.
                // We look at the source text after is_expected to see if it's .to, .to_not, or .not_to.
                let end_offset = loc.end_offset();
                let source_bytes = source.as_bytes();
                let after_is_expected = &source_bytes[end_offset..];

                // Check if the matcher is to_not or not_to (which use should_not)
                let (good, bad) = if after_is_expected.starts_with(b".to_not")
                    || after_is_expected.starts_with(b".not_to")
                {
                    let bad = if let Some(space_pos) = after_is_expected
                        .iter()
                        .position(|&b| b == b' ' || b == b'\n' || b == b'\r')
                    {
                        let matcher = &after_is_expected[..space_pos];
                        format!("`is_expected{}`", String::from_utf8_lossy(matcher))
                    } else {
                        "`is_expected.to`".to_string()
                    };
                    ("`should_not`".to_string(), bad)
                } else {
                    ("`should`".to_string(), "`is_expected.to`".to_string())
                };

                let msg = format!("Prefer {} over {}.", good, bad);
                diagnostics.push(self.diagnostic(source, line, column, msg));
            }
        } else {
            // Default "is_expected" style: flag `should` and `should_not`
            if method_name == b"should" {
                let loc = call.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Prefer `is_expected.to` over `should`.".to_string(),
                ));
            }

            if method_name == b"should_not" {
                let loc = call.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Prefer `is_expected.to_not` over `should_not`.".to_string(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ImplicitExpect, "cops/rspec/implicit_expect");

    #[test]
    fn should_style_flags_is_expected() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("should".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"is_expected.to eq(1)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitExpect, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("should"));
    }

    #[test]
    fn should_style_does_not_flag_should() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("should".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"should eq(1)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitExpect, source, config);
        assert!(diags.is_empty());
    }

    #[test]
    fn should_style_flags_is_expected_to() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("should".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"is_expected.to be_truthy\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitExpect, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("should"));
    }

    #[test]
    fn should_style_flags_is_expected_to_not() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("should".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"is_expected.to_not be_falsy\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitExpect, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("should_not"));
    }

    #[test]
    fn should_style_flags_is_expected_not_to() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("should".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"is_expected.not_to be_falsy\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitExpect, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("should_not"));
    }

    #[test]
    fn should_style_does_not_flag_expect_to() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("should".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"expect(subject).to eq(42)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitExpect, source, config);
        assert!(diags.is_empty());
    }
}
