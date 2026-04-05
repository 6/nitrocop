use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Map of refute_* method names to their assert_not_* counterparts,
/// matching the explicit list in RuboCop's Rails/RefuteMethods cop.
const CORRECTIONS: &[(&str, &str)] = &[
    ("refute", "assert_not"),
    ("refute_empty", "assert_not_empty"),
    ("refute_equal", "assert_not_equal"),
    ("refute_in_delta", "assert_not_in_delta"),
    ("refute_in_epsilon", "assert_not_in_epsilon"),
    ("refute_includes", "assert_not_includes"),
    ("refute_instance_of", "assert_not_instance_of"),
    ("refute_kind_of", "assert_not_kind_of"),
    ("refute_nil", "assert_not_nil"),
    ("refute_operator", "assert_not_operator"),
    ("refute_predicate", "assert_not_predicate"),
    ("refute_respond_to", "assert_not_respond_to"),
    ("refute_same", "assert_not_same"),
    ("refute_match", "assert_no_match"),
];

/// Rails/RefuteMethods: flags `refute_*` methods (or `assert_not_*` with refute style).
///
/// Corpus FN investigation (2 FN in ruby__logger, `refute_predicate` calls):
/// The detection logic already handles all CORRECTIONS entries including `refute_predicate`.
/// The 2 baseline FN were from an older binary that predates the corpus oracle snapshot.
/// Current build detects them correctly when the cop is enabled via baseline config.
/// Added `refute_predicate` fixture coverage to confirm.
pub struct RefuteMethods;

impl Cop for RefuteMethods {
    fn name(&self) -> &'static str {
        "Rails/RefuteMethods"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/test/**/*"]
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
        let style = config.get_str("EnforcedStyle", "assert_not");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        if call.receiver().is_some() {
            return;
        }

        let name = call.name().as_slice();
        let name_str = std::str::from_utf8(name).unwrap_or("");

        let (is_bad, message) = match style {
            "refute" => {
                // Flag assert_not_* methods, suggest refute_*
                // Use the reverse mapping from CORRECTIONS.
                if let Some((refute_name, _)) = CORRECTIONS
                    .iter()
                    .find(|(_, assert_name)| *assert_name == name_str)
                {
                    (true, format!("Prefer `{refute_name}` over `{name_str}`."))
                } else {
                    (false, String::new())
                }
            }
            _ => {
                // "assert_not" (default): flag refute_* methods
                // Only flag methods in the explicit CORRECTIONS list.
                if let Some((_, assert_name)) = CORRECTIONS
                    .iter()
                    .find(|(refute_name, _)| *refute_name == name_str)
                {
                    (true, format!("Prefer `{assert_name}` over `{name_str}`."))
                } else {
                    (false, String::new())
                }
            }
        };

        if !is_bad {
            return;
        }

        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(source, line, column, message));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(RefuteMethods, "cops/rails/refute_methods");

    #[test]
    fn refute_style_flags_assert_not() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("refute".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"assert_not false\n";
        let diags = run_cop_full_with_config(&RefuteMethods, source, config);
        assert!(!diags.is_empty(), "refute style should flag assert_not");
    }

    #[test]
    fn refute_style_allows_refute() {
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("refute".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"refute false\nrefute_empty []\nrefute_equal 1, 2\n";
        assert_cop_no_offenses_full_with_config(&RefuteMethods, source, config);
    }
}
