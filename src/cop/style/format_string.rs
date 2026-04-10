use crate::cop::shared::node_type::{CALL_NODE, INTERPOLATED_STRING_NODE, STRING_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// RuboCop's NodePattern for format/sprintf is `(send nil? :format _ _ ...)` — the
/// `nil?` means it only matches bare calls with no receiver. Previously nitrocop
/// also matched `Kernel.format(...)` and `Kernel.sprintf(...)` via
/// `is_kernel_constant()`, causing 13 FPs in jruby and natalie corpus repos.
///
/// Variant divergence fix: RuboCop's `_ _ ...` pattern requires at least 2 children
/// (arguments). In Prism, shorthand block arguments like `&:inspect` are NOT in
/// `call.arguments()` — they are in `call.block()` as a `BlockArgumentNode`. This
/// caused FN for `format 'text/plain', &:inspect` (1 positional + 1 block arg = 2
/// total) because only the positional count was being checked. Fixed by also
/// counting `BlockArgumentNode` when computing total argument count for
/// `format`/`sprintf` bare calls.
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
                // RuboCop requires at least 2 arguments (including block arguments like &:inspect)
                let positional_count = call
                    .arguments()
                    .map(|a| a.arguments().iter().count())
                    .unwrap_or(0);
                // Block argument (e.g., &:inspect) is NOT in arguments(), it's in call.block()
                let has_block_arg = call
                    .block()
                    .is_some_and(|b| b.as_block_argument_node().is_some());
                let total_count = positional_count + if has_block_arg { 1 } else { 0 };
                if total_count < 2 {
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
                // RuboCop requires at least 2 arguments (including block arguments like &:inspect)
                let positional_count = call
                    .arguments()
                    .map(|a| a.arguments().iter().count())
                    .unwrap_or(0);
                // Block argument (e.g., &:inspect) is NOT in arguments(), it's in call.block()
                let has_block_arg = call
                    .block()
                    .is_some_and(|b| b.as_block_argument_node().is_some());
                let total_count = positional_count + if has_block_arg { 1 } else { 0 };
                if total_count < 2 {
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
    crate::cop_fixture_tests!(FormatString, "cops/style/format_string");

    /// Test that block arguments (e.g., `&:inspect`) are counted towards the
    /// minimum argument count of 2 for format/sprintf detection.
    /// Previously, `format 'text/plain', &:inspect` was missed because the
    /// block argument was not being counted.
    #[test]
    fn format_with_block_argument_flagged_when_style_is_sprintf() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("sprintf".to_string()),
            )]),
            ..CopConfig::default()
        };
        // format with 1 positional arg + 1 block argument = 2 total args
        let source = b"format 'text/plain', &:inspect\n";
        let diags = run_cop_full_with_config(&FormatString, source, config);
        assert!(
            !diags.is_empty(),
            "format with block argument should be flagged when style is sprintf"
        );
    }
}
