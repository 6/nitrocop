use crate::cop::shared::method_dispatch_predicates;
use crate::cop::shared::node_type::{CALL_NODE, NIL_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Matches RuboCop's `RESTRICT_ON_SEND` behavior and ignores safe-navigation
/// calls. Under `EnforcedStyle: comparison`, nitrocop was falsely flagging
/// `region&.nil?` in `twilio__twilio-ruby__42c76f5` at
/// `lib/twilio-ruby/jwt/access_token.rb:82`, but RuboCop only inspects normal
/// `send` nodes for this cop.
pub struct NilComparison;

impl Cop for NilComparison {
    fn name(&self) -> &'static str {
        "Style/NilComparison"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, NIL_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let enforced_style = config.get_str("EnforcedStyle", "predicate");

        let call_node = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_name = call_node.name();
        let method_bytes = method_name.as_slice();

        if call_node.receiver().is_none() {
            return;
        }

        if method_dispatch_predicates::is_safe_navigation(&call_node) {
            return;
        }

        if enforced_style == "predicate" {
            // Flag `x == nil` and `x === nil`
            if method_bytes != b"==" && method_bytes != b"===" {
                return;
            }

            // Check if the argument is nil
            let args = match call_node.arguments() {
                Some(a) => a,
                None => return,
            };
            let arg_list: Vec<_> = args.arguments().iter().collect();
            if arg_list.len() != 1 {
                return;
            }
            if arg_list[0].as_nil_node().is_none() {
                return;
            }

            let msg_loc = call_node
                .message_loc()
                .unwrap_or_else(|| call_node.location());
            let (line, column) = source.offset_to_line_col(msg_loc.start_offset());
            let mut diag = self.diagnostic(
                source,
                line,
                column,
                "Prefer the use of the `nil?` predicate.".to_string(),
            );
            // Autocorrect: replace `x == nil` with `x.nil?`
            // We need to replace from the space before the operator to the end of `nil`
            if let Some(ref mut corr) = corrections {
                let receiver = call_node.receiver().unwrap();
                let receiver_end = receiver.location().end_offset();
                let call_end = call_node.location().end_offset();
                corr.push(crate::correction::Correction {
                    start: receiver_end,
                    end: call_end,
                    replacement: ".nil?".to_string(),
                    cop_name: self.name(),
                    cop_index: 0,
                });
                diag.corrected = true;
            }
            diagnostics.push(diag);
        } else {
            // comparison style: flag `x.nil?`
            if method_bytes != b"nil?" {
                return;
            }

            let msg_loc = call_node
                .message_loc()
                .unwrap_or_else(|| call_node.location());
            let (line, column) = source.offset_to_line_col(msg_loc.start_offset());
            let mut diag = self.diagnostic(
                source,
                line,
                column,
                "Prefer the use of the `==` comparison.".to_string(),
            );
            // Autocorrect: replace `x.nil?` with `x == nil`
            if let Some(ref mut corr) = corrections {
                let receiver = call_node.receiver().unwrap();
                let receiver_end = receiver.location().end_offset();
                let call_end = call_node.location().end_offset();
                corr.push(crate::correction::Correction {
                    start: receiver_end,
                    end: call_end,
                    replacement: " == nil".to_string(),
                    cop_name: self.name(),
                    cop_index: 0,
                });
                diag.corrected = true;
            }
            diagnostics.push(diag);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(NilComparison, "cops/style/nil_comparison");
    crate::cop_autocorrect_fixture_tests!(NilComparison, "cops/style/nil_comparison");

    #[test]
    fn comparison_style_ignores_safe_navigation_nil_checks() {
        let config = CopConfig {
            options: std::collections::HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("comparison".to_string()),
            )]),
            ..CopConfig::default()
        };

        crate::testutil::assert_cop_no_offenses_full_with_config(
            &NilComparison,
            b"headers[:twr] = region unless region&.nil?\n",
            config,
        );
    }
}
