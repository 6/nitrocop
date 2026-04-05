use crate::cop::shared::node_type::CLASS_NODE;
use crate::cop::shared::util::parent_class_name;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

pub struct ActionControllerTestCase;

impl Cop for ActionControllerTestCase {
    fn name(&self) -> &'static str {
        "Rails/ActionControllerTestCase"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/test/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CLASS_NODE]
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
        // minimum_target_rails_version 5.0
        if !config.rails_version_at_least(5.0) {
            return;
        }

        let class = match node.as_class_node() {
            Some(c) => c,
            None => return,
        };

        let parent = match parent_class_name(source, &class) {
            Some(p) => p,
            None => return,
        };

        if parent == b"ActionController::TestCase" {
            let loc = class.class_keyword_loc();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Use `ActionDispatch::IntegrationTest` instead of `ActionController::TestCase`."
                    .to_string(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_rails_fixture_tests!(
        ActionControllerTestCase,
        "cops/rails/action_controller_test_case",
        5.0
    );
}
