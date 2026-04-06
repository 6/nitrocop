use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::shared::util::is_dsl_call;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-07)
///
/// FP=1, FN=0 per corpus oracle. Could not reproduce locally — check-cop.py
/// shows PASS (0 excess). The single FP was likely a config-dependent artifact
/// (e.g., repo-specific Exclude/Include overrides not replicated locally).
///
/// ## Investigation (2026-03-15, round 2)
///
/// **FP root cause (1 FP):** Bare validator calls with no arguments
/// (e.g., `validates_numericality_of` with no field name) were flagged.
/// RuboCop's `on_send` returns early with `return unless node.last_argument`.
/// Fix: added `call.arguments().is_some()` check before flagging.
///
/// ## Investigation (2026-03-16, round 3)
///
/// **FP root cause (1 FP):** `validates_presence_of field` where `field` is a
/// local variable (block parameter from `.each`) was flagged. RuboCop skips
/// the offense when the last argument is not a literal, splat, or frozen array
/// (see `on_send` corrector guard and spec cases for trailing send/variable/
/// constant). Fix: after confirming arguments exist, check that the last
/// argument is a literal-like node type (symbol, string, integer, array, hash,
/// keyword hash, splat, etc.). Skip when it's a call node, local variable read,
/// or constant — matching RuboCop's behavior.
pub struct Validation;

const OLD_VALIDATORS: &[(&[u8], &str)] = &[
    (b"validates_presence_of", "presence: true"),
    (b"validates_uniqueness_of", "uniqueness: true"),
    (b"validates_format_of", "format: { ... }"),
    (b"validates_length_of", "length: { ... }"),
    (b"validates_inclusion_of", "inclusion: { ... }"),
    (b"validates_exclusion_of", "exclusion: { ... }"),
    (b"validates_numericality_of", "numericality: true"),
    (b"validates_acceptance_of", "acceptance: true"),
    (b"validates_confirmation_of", "confirmation: true"),
    (b"validates_size_of", "length: { ... }"),
    (b"validates_comparison_of", "comparison: { ... }"),
    (b"validates_absence_of", "absence: true"),
];

impl Cop for Validation {
    fn name(&self) -> &'static str {
        "Rails/Validation"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/app/models/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };

        // RuboCop skips the offense when the last argument is not a literal,
        // splat, or frozen array (e.g., a method call, local variable, or
        // constant). This prevents false positives on dynamic usage like
        // `validates_presence_of field` where `field` is a variable.
        let arg_list: Vec<_> = args.arguments().iter().collect();
        if let Some(last) = arg_list.last() {
            if !is_literal_like(last) {
                return;
            }
        }

        for &(old_name, replacement) in OLD_VALIDATORS {
            if is_dsl_call(&call, old_name) {
                let loc = call.message_loc().unwrap_or(call.location());
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                let old_str = String::from_utf8_lossy(old_name);
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!("Use `validates :attr, {replacement}` instead of `{old_str}`."),
                ));
            }
        }
    }
}

/// Returns true if the node is a "literal-like" type that RuboCop considers
/// valid as the last argument to old-style validators. This includes symbol,
/// string, integer, float, array, hash, keyword hash, splat, range, regex,
/// nil, true, false, and lambda/block-pass nodes.
fn is_literal_like(node: &ruby_prism::Node<'_>) -> bool {
    node.as_symbol_node().is_some()
        || node.as_string_node().is_some()
        || node.as_integer_node().is_some()
        || node.as_float_node().is_some()
        || node.as_array_node().is_some()
        || node.as_hash_node().is_some()
        || node.as_keyword_hash_node().is_some()
        || node.as_splat_node().is_some()
        || node.as_range_node().is_some()
        || node.as_regular_expression_node().is_some()
        || node.as_nil_node().is_some()
        || node.as_true_node().is_some()
        || node.as_false_node().is_some()
        || node.as_lambda_node().is_some()
        || node.as_block_argument_node().is_some()
        || node.as_interpolated_string_node().is_some()
        || node.as_interpolated_symbol_node().is_some()
        // frozen array: `[:a, :b].freeze` is a CallNode on an array receiver
        || is_frozen_array(node)
}

/// Checks for the `[...].freeze` pattern (frozen array literal).
fn is_frozen_array(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        if call.name().as_slice() == b"freeze" {
            if let Some(recv) = call.receiver() {
                return recv.as_array_node().is_some();
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(Validation, "cops/rails/validation");
}
