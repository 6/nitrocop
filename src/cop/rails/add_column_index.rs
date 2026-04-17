use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::shared::util::{keyword_arg_pair_start_offset, keyword_arg_value};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Rails/AddColumnIndex: flags `add_column` calls that pass an `index:` keyword.
///
/// RuboCop reports this offense on the `index:` pair itself, not at the start of
/// the `add_column` call. Nitrocop was previously reporting multiline calls on the
/// first line, which produced FP/FN location swaps in corpus repos such as
/// Coursemology, Conjur, and Pupilfirst. RuboCop also includes all `db/**/*.rb`
/// files for this cop, so `db/old_migrations/...` must stay in scope too.
pub struct AddColumnIndex;

impl Cop for AddColumnIndex {
    fn name(&self) -> &'static str {
        "Rails/AddColumnIndex"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["db/**/*.rb"]
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

        let offset = keyword_arg_pair_start_offset(&call, b"index")
            .unwrap_or_else(|| node.location().start_offset());
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

    #[test]
    fn default_include_matches_rubocop() {
        assert_eq!(AddColumnIndex.default_include(), &["db/**/*.rb"]);
    }
}
