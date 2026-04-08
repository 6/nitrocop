use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::shared::util::{keyword_arg_pair_start_offset, keyword_arg_value};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Detects `add_column` calls that incorrectly pass an `index:` option.
///
/// ## Investigation findings (2026-04-08)
///
/// Corpus FN investigation found one real detection bug: multiline `add_column`
/// calls were reported at the call start even when the `index:` keyword pair
/// lived on the following line. The corpus oracle compares by `(file, line)`,
/// and RuboCop anchors the offense on the `index:` pair, so those continuation
/// lines were counted as false negatives. Fix: when the `index:` pair starts on
/// a later line than the `add_column` call, report at the pair instead.
pub struct AddColumnIndex;

impl Cop for AddColumnIndex {
    fn name(&self) -> &'static str {
        "Rails/AddColumnIndex"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["db/migrate/**/*.rb"]
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

        if call.name().as_slice() != b"add_column" {
            return;
        }

        // Check if there's an `index` keyword argument
        if keyword_arg_value(&call, b"index").is_none() {
            return;
        }

        let call_start = node.location().start_offset();
        let call_line = source.offset_to_line_col(call_start).0;
        let offset = keyword_arg_pair_start_offset(&call, b"index")
            .filter(|pair_start| source.offset_to_line_col(*pair_start).0 != call_line)
            .unwrap_or(call_start);
        let (line, column) = source.offset_to_line_col(offset);
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            "`add_column` does not accept an `index` key, use `add_index` instead.".to_string(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(AddColumnIndex, "cops/rails/add_column_index");
}
