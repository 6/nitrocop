use crate::cop::rspec_rails::RSPEC_RAILS_DEFAULT_INCLUDE;
use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// RuboCop's `not_to?` node matcher pattern only matches `:not_to`, not `:to_not`.
/// In `be_invalid` style, `expect(foo).to_not be_valid` must NOT be flagged —
/// only `expect(foo).not_to be_valid` is an offense. This matches RuboCop's behavior.
pub struct NegationBeValid;

impl Cop for NegationBeValid {
    fn name(&self) -> &'static str {
        "RSpecRails/NegationBeValid"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_RAILS_DEFAULT_INCLUDE
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
        let enforced_style = config.get_str("EnforcedStyle", "not_to");

        // Look for runner calls: to/not_to/to_not
        let runner_call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let runner_name = runner_call.name().as_slice();
        let is_to = runner_name == b"to";
        let is_not_to = runner_name == b"not_to" || runner_name == b"to_not";

        if !is_to && !is_not_to {
            return;
        }

        // Verify receiver is expect(...) or is_expected
        let recv = match runner_call.receiver() {
            Some(r) => r,
            None => return,
        };
        let expect_call = match recv.as_call_node() {
            Some(c) => c,
            None => return,
        };
        let expect_name = expect_call.name().as_slice();
        let is_expect = expect_name == b"expect" && expect_call.receiver().is_none();
        let is_is_expected = expect_name == b"is_expected"
            && expect_call.receiver().is_none()
            && expect_call.arguments().is_none();
        if !is_expect && !is_is_expected {
            return;
        }

        // Get the matcher argument
        let args = match runner_call.arguments() {
            Some(a) => a,
            None => return,
        };
        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.len() != 1 {
            return;
        }

        let matcher = match arg_list[0].as_call_node() {
            Some(c) => c,
            None => return,
        };

        let matcher_name = matcher.name().as_slice();

        // Must be be_valid or be_invalid, with no receiver (bare matcher call)
        if matcher_name != b"be_valid" && matcher_name != b"be_invalid" {
            return;
        }

        if matcher.receiver().is_some() {
            return;
        }

        match enforced_style {
            "not_to" => {
                // Flag: expect(x).to be_invalid -> suggest expect(x).not_to be_valid
                if is_to && matcher_name == b"be_invalid" {
                    let runner_loc = runner_call.message_loc().unwrap_or(runner_call.location());
                    let (line, column) = source.offset_to_line_col(runner_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `expect(...).not_to be_valid`.".to_string(),
                    ));
                }
            }
            "be_invalid" => {
                // Flag: expect(x).not_to be_valid -> suggest expect(x).to be_invalid
                // RuboCop's not_to? pattern only matches :not_to, not :to_not
                if runner_name == b"not_to" && matcher_name == b"be_valid" {
                    let runner_loc = runner_call.message_loc().unwrap_or(runner_call.location());
                    let (line, column) = source.offset_to_line_col(runner_loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `expect(...).to be_invalid`.".to_string(),
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
    crate::cop_fixture_tests!(NegationBeValid, "cops/rspecrails/negation_be_valid");

    #[test]
    fn be_invalid_style_flags_not_to_be_valid() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("be_invalid".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"expect(foo).not_to be_valid\n";
        let diags = run_cop_full_with_config(&NegationBeValid, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("be_invalid"));
    }

    #[test]
    fn be_invalid_style_allows_to_not_be_valid() {
        // RuboCop's not_to? pattern only matches :not_to, not :to_not
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("be_invalid".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"expect(foo).to_not be_valid\n";
        assert_cop_no_offenses_full_with_config(&NegationBeValid, source, config);
    }

    #[test]
    fn be_invalid_style_allows_to_be_invalid() {
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("be_invalid".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"expect(foo).to be_invalid\n";
        assert_cop_no_offenses_full_with_config(&NegationBeValid, source, config);
    }
}
