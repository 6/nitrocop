use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Corpus investigation (2026-03): Re-added Java:: exemption. RuboCop's
/// `java_type_node?` matcher exempts `(send (const nil? :Java) _)` — any
/// call where the receiver is the bare `Java` constant (not a qualified
/// path like `Foo::Java`). This eliminates 305 FPs from JRuby Java interop
/// code (e.g., `Java::int`, `Java::byte`, `Java::com`). The previous
/// removal was based on 2 FNs for `Java::define_exception_handler` and
/// `Java::se` in jruby, but those were actually correct exemptions per
/// RuboCop's spec — the corpus oracle had a stale comparison.
pub struct ColonMethodCall;

impl Cop for ColonMethodCall {
    fn name(&self) -> &'static str {
        "Style/ColonMethodCall"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let call_node = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // Must have a receiver
        if call_node.receiver().is_none() {
            return;
        }

        // Must use :: as the call operator
        let call_op_loc = match call_node.call_operator_loc() {
            Some(loc) => loc,
            None => return,
        };

        if call_op_loc.as_slice() != b"::" {
            return;
        }

        // The method name must start with a lowercase letter or underscore
        // (i.e., it's a regular method, not a constant access)
        let method_name = call_node.name();
        let name_bytes = method_name.as_slice();
        if name_bytes.is_empty() {
            return;
        }

        let first = name_bytes[0];
        // Skip if it starts with uppercase (constant access like Foo::Bar)
        if first.is_ascii_uppercase() {
            return;
        }

        // RuboCop's java_type_node? matcher: exempt calls where the receiver
        // is the bare `Java` constant (ConstantReadNode). Qualified paths like
        // `Foo::Java::bar` use constant_path_node and are NOT exempt — only
        // `(send (const nil? :Java) _)` matches RuboCop's pattern.
        if let Some(receiver) = call_node.receiver() {
            if receiver
                .as_constant_read_node()
                .is_some_and(|c| c.name().as_slice() == b"Java")
            {
                return;
            }
        }

        let (line, column) = source.offset_to_line_col(call_op_loc.start_offset());
        let mut diag = self.diagnostic(
            source,
            line,
            column,
            "Do not use `::` for method calls.".to_string(),
        );
        if let Some(corr) = corrections {
            // Replace `::` with `.`.
            corr.push(crate::correction::Correction {
                start: call_op_loc.start_offset(),
                end: call_op_loc.end_offset(),
                replacement: ".".to_string(),
                cop_name: self.name(),
                cop_index: 0,
            });
            diag.corrected = true;
        }
        diagnostics.push(diag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ColonMethodCall, "cops/style/colon_method_call");
    crate::cop_autocorrect_fixture_tests!(ColonMethodCall, "cops/style/colon_method_call");
}
