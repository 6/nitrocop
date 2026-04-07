use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// RSpec/ImplicitExpect cop.
///
/// Flags `should` or `should_not` in favor of `is_expected.to`/`is_expected.to_not`
/// when `EnforcedStyle: is_expected` (default), and flags `is_expected.to`/`is_expected.to_not`/`is_expected.not_to`
/// in favor of `should`/`should_not` when `EnforcedStyle: should`.
///
/// In "should" style, only the Runner level (`is_expected.to`, `is_expected.to_not`, `is_expected.not_to`)
/// is flagged — not bare `is_expected` — to avoid duplicate offenses and produce correct messages.
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

        let method_name = call.name().as_slice();

        if enforced_style == "should" {
            // "should" style: flag `is_expected.to`, `is_expected.to_not`, `is_expected.not_to`
            // Only flag at the Runner level - not bare `is_expected`
            let receiver = call.receiver();
            let is_runner =
                method_name == b"to" || method_name == b"to_not" || method_name == b"not_to";

            if is_runner {
                if let Some(receiver_node) = receiver {
                    // Check if receiver is `is_expected` with no receiver itself
                    if let Some(receiver_call) = receiver_node.as_call_node() {
                        if receiver_call.name().as_slice() == b"is_expected"
                            && receiver_call.receiver().is_none()
                        {
                            // Flag at the is_expected position (the start of the expression)
                            let loc = receiver_node.location();
                            let (line, column) = source.offset_to_line_col(loc.start_offset());
                            let msg = match method_name {
                                b"to" => "Prefer `should` over `is_expected.to`.",
                                b"to_not" => "Prefer `should` over `is_expected.to_not`.",
                                _ => "Prefer `should` over `is_expected.not_to`.",
                            };
                            diagnostics.push(self.diagnostic(
                                source,
                                line,
                                column,
                                msg.to_string(),
                            ));
                        }
                    }
                }
            }
            // NOTE: In "should" style, bare `is_expected` (without Runner) is NOT flagged
            // because `is_expected` alone is not an offense - only `is_expected.to` etc. are
        } else {
            // Default "is_expected" style: flag `should` and `should_not`
            if call.receiver().is_none() {
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
}

fn should_style_config() -> crate::cop::CopConfig {
    use std::collections::HashMap;
    crate::cop::CopConfig {
        options: HashMap::from([(
            "EnforcedStyle".into(),
            serde_yml::Value::String("should".into()),
        )]),
        ..crate::cop::CopConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ImplicitExpect, "cops/rspec/implicit_expect");

    #[test]
    fn should_style_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ImplicitExpect,
            include_bytes!(
                "../../../tests/fixtures/cops/rspec/implicit_expect/should_style_offense.rb"
            ),
            should_style_config(),
        );
    }

    #[test]
    fn should_style_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ImplicitExpect,
            include_bytes!(
                "../../../tests/fixtures/cops/rspec/implicit_expect/should_style_no_offense.rb"
            ),
            should_style_config(),
        );
    }

    #[test]
    fn should_style_flags_is_expected() {
        let source = b"is_expected.to eq(1)\n";
        let diags = crate::testutil::run_cop_full_with_config(
            &ImplicitExpect,
            source,
            should_style_config(),
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("should"));
    }

    #[test]
    fn should_style_does_not_flag_should() {
        let source = b"should eq(1)\n";
        let diags = crate::testutil::run_cop_full_with_config(
            &ImplicitExpect,
            source,
            should_style_config(),
        );
        assert!(diags.is_empty());
    }
}
