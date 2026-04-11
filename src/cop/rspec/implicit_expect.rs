use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

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
            // "should" style: check .to/.not_to/.to_not calls with is_expected receiver.
            // Mirrors RuboCop's RESTRICT_ON_SEND = Runners.all, which only triggers on
            // :to/:not_to/:to_not. The cop then extracts `source_range.source` and looks
            // it up in ENFORCED_REPLACEMENTS. If the source has unusual whitespace
            // (multiline or extra spaces), `Hash#fetch` raises KeyError and the offense
            // is silently skipped — we replicate this quirk.
            if !matches!(method_name, b"to" | b"not_to" | b"to_not") {
                return;
            }
            let receiver = match call.receiver() {
                Some(r) => r,
                None => return,
            };
            let recv_call = match receiver.as_call_node() {
                Some(c) => c,
                None => return,
            };
            if recv_call.receiver().is_some() || recv_call.name().as_slice() != b"is_expected" {
                return;
            }

            // Verify source text matches exactly (no whitespace anomalies)
            let is_expected_start = recv_call.location().start_offset();
            let msg_loc = match call.message_loc() {
                Some(loc) => loc,
                None => return,
            };
            let source_text = &source.as_bytes()[is_expected_start..msg_loc.end_offset()];

            let (expected_source, replacement): (&[u8], &str) = match method_name {
                b"to" => (b"is_expected.to", "should"),
                b"not_to" => (b"is_expected.not_to", "should_not"),
                b"to_not" => (b"is_expected.to_not", "should_not"),
                _ => unreachable!(),
            };

            if source_text != expected_source {
                return; // Multiline or whitespace anomaly — RuboCop silently skips these
            }

            let bad = std::str::from_utf8(expected_source).unwrap();
            let (line, column) = source.offset_to_line_col(is_expected_start);
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                format!("Prefer `{replacement}` over `{bad}`."),
            ));
        } else {
            // Default "is_expected" style: flag `should` and `should_not`
            if call.receiver().is_some() {
                return;
            }

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

    fn should_config() -> crate::cop::CopConfig {
        use std::collections::HashMap;
        crate::cop::CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("should".into()),
            )]),
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn should_style_offense_fixture() {
        let fixture =
            include_bytes!("../../../tests/fixtures/cops/rspec/implicit_expect/should_offense.rb");
        let fixture_str = std::str::from_utf8(fixture).expect("fixture must be valid UTF-8");
        let source = fixture_str
            .strip_prefix("# nitrocop-config: EnforcedStyle: should\n")
            .expect("fixture should start with should config directive");
        crate::testutil::assert_cop_offenses_full_with_config(
            &ImplicitExpect,
            source.as_bytes(),
            should_config(),
        );
    }

    #[test]
    fn should_style_no_offense_fixture() {
        let fixture = include_bytes!(
            "../../../tests/fixtures/cops/rspec/implicit_expect/should_no_offense.rb"
        );
        let fixture_str = std::str::from_utf8(fixture).expect("fixture must be valid UTF-8");
        let source = fixture_str
            .strip_prefix("# nitrocop-config: EnforcedStyle: should\n")
            .expect("fixture should start with should config directive");
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ImplicitExpect,
            source.as_bytes(),
            should_config(),
        );
    }

    #[test]
    fn should_style_flags_is_expected() {
        let source = b"is_expected.to eq(1)\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&ImplicitExpect, source, should_config());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("should"));
    }

    #[test]
    fn should_style_does_not_flag_should() {
        let source = b"should eq(1)\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&ImplicitExpect, source, should_config());
        assert!(diags.is_empty());
    }
}
