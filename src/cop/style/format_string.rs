use crate::cop::shared::node_type::{CALL_NODE, INTERPOLATED_STRING_NODE, STRING_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// RuboCop's NodePattern for format/sprintf is `(send nil? :format _ _ ...)` —
/// the `nil?` means it only matches bare calls with no receiver. Previously
/// nitrocop also matched `Kernel.format(...)` and `Kernel.sprintf(...)` via
/// `is_kernel_constant()`, causing 13 FPs in jruby and natalie corpus repos.
/// Fixed by requiring `receiver().is_none()` for format/sprintf.
///
/// Variant style tests (sprintf, percent) were added to verify correct behavior
/// across all EnforcedStyle values. The implementation correctly handles:
/// - `format`/`sprintf` with 2+ args for all styles
/// - `String#%` with string receiver (not flagged when style=percent)
/// - `String#%` with non-string receiver and array/hash arg (not flagged when style=percent)
pub struct FormatString;

impl Cop for FormatString {
    fn name(&self) -> &'static str {
        "Style/FormatString"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, INTERPOLATED_STRING_NODE, STRING_NODE]
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
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_bytes = call.name().as_slice();
        let style = config.get_str("EnforcedStyle", "format");

        match method_bytes {
            b"%" => {
                // String#% - only flag when style prefers format or sprintf
                if style == "percent" {
                    return;
                }
                // Must have a non-nil receiver
                let receiver = match call.receiver() {
                    Some(r) => r,
                    None => return,
                };

                let is_string_receiver = receiver.as_string_node().is_some()
                    || receiver.as_interpolated_string_node().is_some();

                if !is_string_receiver {
                    // For non-string receivers, only flag when RHS is an array or hash literal
                    // RuboCop pattern: (send !nil? $:% {array hash})
                    let has_array_or_hash_arg = call.arguments().is_some_and(|args| {
                        let arg_list: Vec<_> = args.arguments().iter().collect();
                        arg_list.len() == 1
                            && (arg_list[0].as_array_node().is_some()
                                || arg_list[0].as_hash_node().is_some()
                                || arg_list[0].as_keyword_hash_node().is_some())
                    });
                    if !has_array_or_hash_arg {
                        return;
                    }
                }

                // RuboCop points at the % operator (node.loc.selector), not the whole expression
                let loc = call.message_loc().unwrap_or_else(|| call.location());
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                let preferred = if style == "format" {
                    "format"
                } else {
                    "sprintf"
                };
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!("Favor `{}` over `String#%`.", preferred),
                ));
            }
            b"format" => {
                if style == "format" {
                    return;
                }
                // RuboCop pattern: (send nil? :format _ _ ...) — only bare calls
                if call.receiver().is_some() {
                    return;
                }
                // RuboCop requires at least 2 arguments
                let arg_count = call
                    .arguments()
                    .map(|a| a.arguments().iter().count())
                    .unwrap_or(0);
                if arg_count < 2 {
                    return;
                }

                // RuboCop points at the method name (node.loc.selector), not the whole expression
                let loc = call.message_loc().unwrap_or_else(|| call.location());
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                let preferred = if style == "sprintf" {
                    "sprintf"
                } else {
                    "String#%"
                };
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!("Favor `{}` over `format`.", preferred),
                ));
            }
            b"sprintf" => {
                if style == "sprintf" {
                    return;
                }
                // RuboCop pattern: (send nil? :sprintf _ _ ...) — only bare calls
                if call.receiver().is_some() {
                    return;
                }
                // RuboCop requires at least 2 arguments
                let arg_count = call
                    .arguments()
                    .map(|a| a.arguments().iter().count())
                    .unwrap_or(0);
                if arg_count < 2 {
                    return;
                }

                // RuboCop points at the method name (node.loc.selector), not the whole expression
                let loc = call.message_loc().unwrap_or_else(|| call.location());
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                let preferred = if style == "format" {
                    "format"
                } else {
                    "String#%"
                };
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!("Favor `{}` over `sprintf`.", preferred),
                ));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{
        assert_cop_no_offenses_full_with_config, assert_cop_offenses_full_with_config,
    };

    crate::cop_fixture_tests!(FormatString, "cops/style/format_string");

    fn sprintf_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("sprintf".into()),
            )]),
            ..CopConfig::default()
        }
    }

    fn percent_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("percent".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn sprintf_offense_fixture() {
        assert_cop_offenses_full_with_config(
            &FormatString,
            include_bytes!("../../../tests/fixtures/cops/style/format_string/sprintf_offense.rb"),
            sprintf_config(),
        );
    }

    #[test]
    fn sprintf_no_offense_fixture() {
        assert_cop_no_offenses_full_with_config(
            &FormatString,
            include_bytes!(
                "../../../tests/fixtures/cops/style/format_string/sprintf_no_offense.rb"
            ),
            sprintf_config(),
        );
    }

    #[test]
    fn percent_offense_fixture() {
        assert_cop_offenses_full_with_config(
            &FormatString,
            include_bytes!("../../../tests/fixtures/cops/style/format_string/percent_offense.rb"),
            percent_config(),
        );
    }

    #[test]
    fn percent_no_offense_fixture() {
        assert_cop_no_offenses_full_with_config(
            &FormatString,
            include_bytes!(
                "../../../tests/fixtures/cops/style/format_string/percent_no_offense.rb"
            ),
            percent_config(),
        );
    }
}
