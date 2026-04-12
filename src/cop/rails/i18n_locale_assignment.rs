use crate::cop::shared::constant_predicates;
use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-04-12)
///
/// FP=62, FN=0. The false positives were namespaced constants such as
/// `Pagy::I18n.locale = ...` and `Mongoid::Fields::I18n.locale = ...`.
/// RuboCop only matches `(const {nil? cbase} :I18n)`, so qualified paths must
/// be excluded even when their final segment is `I18n`.
pub struct I18nLocaleAssignment;

impl Cop for I18nLocaleAssignment {
    fn name(&self) -> &'static str {
        "Rails/I18nLocaleAssignment"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["spec/**/*.rb", "test/**/*.rb"]
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

        if call.name().as_slice() != b"locale=" {
            return;
        }

        let recv = match call.receiver() {
            Some(r) => r,
            None => return,
        };

        // RuboCop matches only bare `I18n` or root-qualified `::I18n`,
        // not namespaced constants like `Pagy::I18n`.
        if !constant_predicates::is_simple_constant(&recv, b"I18n") {
            return;
        }

        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            "Use `I18n.with_locale` instead of directly setting `I18n.locale`.".to_string(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(I18nLocaleAssignment, "cops/rails/i18n_locale_assignment");
}
