use crate::cop::shared::node_type::{CALL_NODE, GLOBAL_VARIABLE_READ_NODE, INTEGER_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/NumericPredicate: checks for comparison operators used to test numbers
/// as zero, positive, or negative, suggesting predicate methods instead.
///
/// FP fix (predicate style): safe navigation calls (`&.>`, `&.<`, `&.==`) must be
/// skipped because RuboCop's NodePattern only matches `send` nodes, not `csend`.
///
/// FP fix (comparison style): safe navigation calls (`&.zero?`, `&.positive?`,
/// `&.negative?`) must be skipped for the same reason — RuboCop uses `on_send`
/// which does not fire for `csend` (safe navigation) nodes.
///
/// FN fix: hex (0x00), binary (0b0000), and octal (0o0) integer literals were not
/// recognized as zero because the source text was parsed with `str::parse::<i64>()`
/// which doesn't handle Ruby's numeric prefixes. Now uses `i64::from_str_radix`.
///
/// FN fix: Prism sets `call_operator_loc` for regular dotted operator-method calls
/// like `foo.>(0)` and `s[:fee].>(0)` to `.`. The previous implementation treated
/// any `call_operator_loc` as safe navigation and skipped these legitimate `send`
/// nodes. Now only `&.` is skipped, matching RuboCop's `send` vs `csend` split.
pub struct NumericPredicate;

impl NumericPredicate {
    fn int_value(node: &ruby_prism::Node<'_>) -> Option<i64> {
        if let Some(int_node) = node.as_integer_node() {
            let src = int_node.location().as_slice();
            if let Ok(s) = std::str::from_utf8(src) {
                // Strip underscores (Ruby allows 1_000_000)
                let cleaned = s.replace('_', "");
                // Handle hex, binary, octal prefixes
                if let Some(hex) = cleaned
                    .strip_prefix("0x")
                    .or_else(|| cleaned.strip_prefix("0X"))
                {
                    return i64::from_str_radix(hex, 16).ok();
                }
                if let Some(bin) = cleaned
                    .strip_prefix("0b")
                    .or_else(|| cleaned.strip_prefix("0B"))
                {
                    return i64::from_str_radix(bin, 2).ok();
                }
                if let Some(oct) = cleaned
                    .strip_prefix("0o")
                    .or_else(|| cleaned.strip_prefix("0O"))
                {
                    return i64::from_str_radix(oct, 8).ok();
                }
                // Handle negative sign
                if let Some(rest) = cleaned.strip_prefix('-') {
                    return rest.parse::<i64>().ok().map(|v| -v);
                }
                return cleaned.parse::<i64>().ok();
            }
        }
        None
    }

    fn is_global_var(node: &ruby_prism::Node<'_>) -> bool {
        node.as_global_variable_read_node().is_some()
    }

    /// Matches RuboCop's `parenthesized_source`: wraps the receiver in parens
    /// when it's a binary operation (operator method where the expression starts
    /// before the selector, e.g. `cmd >> 4`, `r_val[0]`).
    fn parenthesized_source(node: &ruby_prism::Node<'_>) -> String {
        let src = std::str::from_utf8(node.location().as_slice()).unwrap_or("x");
        if Self::requires_parentheses(node) {
            format!("({})", src)
        } else {
            src.to_string()
        }
    }

    fn requires_parentheses(node: &ruby_prism::Node<'_>) -> bool {
        if let Some(call) = node.as_call_node() {
            let method = call.name();
            let method_bytes = method.as_slice();
            // Check if this is an operator method
            let is_operator = matches!(
                method_bytes,
                b"|" | b"^"
                    | b"&"
                    | b"<=>"
                    | b"=="
                    | b"==="
                    | b"=~"
                    | b">"
                    | b">="
                    | b"<"
                    | b"<="
                    | b"<<"
                    | b">>"
                    | b"+"
                    | b"-"
                    | b"*"
                    | b"/"
                    | b"%"
                    | b"**"
                    | b"[]"
                    | b"[]="
            );
            if !is_operator {
                return false;
            }
            // Binary operation: expression start differs from message (selector) start
            if let Some(msg_loc) = call.message_loc() {
                let expr_start = node.location().start_offset();
                let msg_start = msg_loc.start_offset();
                if expr_start == msg_start {
                    return false; // unary operation
                }
                // Check if already parenthesized (opening_loc present means `foo.(bar)`)
                // For our purposes, check if the source is wrapped in parens
                let src = node.location().as_slice();
                if src.first() == Some(&b'(') && src.last() == Some(&b')') {
                    return false;
                }
                return true;
            }
        }
        false
    }
}

impl Cop for NumericPredicate {
    fn name(&self) -> &'static str {
        "Style/NumericPredicate"
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["spec/**/*"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, GLOBAL_VARIABLE_READ_NODE, INTEGER_NODE]
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
        let _allowed_methods = config.get_string_array("AllowedMethods");
        let _allowed_patterns = config.get_string_array("AllowedPatterns");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_name = call.name();
        let method_bytes = method_name.as_slice();

        if enforced_style == "predicate" {
            // Check for: x == 0, x > 0, x < 0, 0 == x, 0 > x, 0 < x
            if !matches!(method_bytes, b"==" | b">" | b"<") {
                return;
            }

            // Skip safe navigation calls (x&.>(0), x&.==(0)) — RuboCop only
            // matches `send` nodes, not `csend` (safe navigation).
            if call
                .call_operator_loc()
                .is_some_and(|loc| loc.as_slice() == b"&.")
            {
                return;
            }

            if let Some(args) = call.arguments() {
                let arg_list: Vec<_> = args.arguments().iter().collect();
                if arg_list.len() != 1 {
                    return;
                }

                if let Some(receiver) = call.receiver() {
                    // x == 0, x > 0, x < 0
                    if Self::int_value(&arg_list[0]) == Some(0) && !Self::is_global_var(&receiver) {
                        let recv_src = Self::parenthesized_source(&receiver);
                        let replacement = match method_bytes {
                            b"==" => format!("{}.zero?", recv_src),
                            b">" => format!("{}.positive?", recv_src),
                            b"<" => format!("{}.negative?", recv_src),
                            _ => return,
                        };
                        let loc = node.location();
                        let current = std::str::from_utf8(loc.as_slice()).unwrap_or("");
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            format!("Use `{}` instead of `{}`.", replacement, current),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: loc.start_offset(),
                                end: loc.end_offset(),
                                replacement: replacement.clone(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }

                    // 0 == x, 0 > x, 0 < x (inverted)
                    if Self::int_value(&receiver) == Some(0) && !Self::is_global_var(&arg_list[0]) {
                        let arg_src = Self::parenthesized_source(&arg_list[0]);
                        let replacement = match method_bytes {
                            b"==" => format!("{}.zero?", arg_src),
                            b">" => format!("{}.negative?", arg_src), // 0 > x means x is negative
                            b"<" => format!("{}.positive?", arg_src), // 0 < x means x is positive
                            _ => return,
                        };
                        let loc = node.location();
                        let current = std::str::from_utf8(loc.as_slice()).unwrap_or("");
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            format!("Use `{}` instead of `{}`.", replacement, current),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: loc.start_offset(),
                                end: loc.end_offset(),
                                replacement: replacement.clone(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                }
            }
        } else if enforced_style == "comparison" {
            // Check for: x.zero?, x.positive?, x.negative?
            if !matches!(method_bytes, b"zero?" | b"positive?" | b"negative?") {
                return;
            }
            // Skip safe navigation calls (x&.zero?, x&.positive?) — RuboCop only
            // matches `send` nodes, not `csend` (safe navigation).
            if call
                .call_operator_loc()
                .is_some_and(|loc| loc.as_slice() == b"&.")
            {
                return;
            }
            if call.arguments().is_some() {
                return;
            }
            if let Some(receiver) = call.receiver() {
                let recv_src = std::str::from_utf8(receiver.location().as_slice()).unwrap_or("x");
                let replacement = match method_bytes {
                    b"zero?" => format!("{} == 0", recv_src),
                    b"positive?" => format!("{} > 0", recv_src),
                    b"negative?" => format!("{} < 0", recv_src),
                    _ => return,
                };
                let loc = node.location();
                let current = std::str::from_utf8(loc.as_slice()).unwrap_or("");
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                let mut diag = self.diagnostic(
                    source,
                    line,
                    column,
                    format!("Use `{}` instead of `{}`.", replacement, current),
                );
                if let Some(ref mut corr) = corrections {
                    corr.push(crate::correction::Correction {
                        start: loc.start_offset(),
                        end: loc.end_offset(),
                        replacement: replacement.clone(),
                        cop_name: self.name(),
                        cop_index: 0,
                    });
                    diag.corrected = true;
                }
                diagnostics.push(diag);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(NumericPredicate, "cops/style/numeric_predicate");
    crate::cop_autocorrect_fixture_tests!(NumericPredicate, "cops/style/numeric_predicate");
}
