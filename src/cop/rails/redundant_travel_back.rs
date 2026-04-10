use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Matches RuboCop's `Rails/RedundantTravelBack` for Rails 5.2+ test files.
///
/// Corpus fixes for this cop fell into three buckets:
///
/// - The fallback include list has to cover both `spec/**/*.rb` and
///   `test/**/*.rb`, because RuboCop's default config includes both.
/// - `after` matching must allow receivers such as `config.after`, because
///   RuboCop treats any block method named `after` as eligible.
/// - The Rails version gate must still go through `rails_version_at_least(5.2)`,
///   not `target_rails_version()` directly. RuboCop's
///   `minimum_target_rails_version 5.2` also implies `requires_gem 'railties'`,
///   so repos with `TargetRailsVersion` set but no `railties` in `Gemfile.lock`
///   must be skipped. Bypassing that gate caused the reported false positives.
///
pub struct RedundantTravelBack;

impl Cop for RedundantTravelBack {
    fn name(&self) -> &'static str {
        "Rails/RedundantTravelBack"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/spec/**/*.rb", "**/test/**/*.rb"]
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::cop::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // minimum_target_rails_version 5.2
        if !config.rails_version_at_least(5.2) {
            return;
        }

        let mut visitor = TravelBackVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            in_teardown_or_after: false,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct TravelBackVisitor<'a> {
    cop: &'a RedundantTravelBack,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    in_teardown_or_after: bool,
}

impl<'a, 'pr> Visit<'pr> for TravelBackVisitor<'a> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let method_name = node.name().as_slice();

        // Check if we're entering an `after` block.
        // RuboCop only matches method defs named `teardown` and block calls
        // named `after`; `teardown do ... end` blocks are not flagged.
        let enters_after = method_name == b"after"
            && node
                .block()
                .and_then(|block| block.as_block_node())
                .is_some();

        // Check if this is a `travel_back` call inside teardown/after
        if self.in_teardown_or_after && method_name == b"travel_back" && node.receiver().is_none() {
            let loc = node.location();
            let (line, column) = self.source.offset_to_line_col(loc.start_offset());
            self.diagnostics.push(
                self.cop.diagnostic(
                    self.source,
                    line,
                    column,
                    "Redundant `travel_back` detected. It is automatically called after each test."
                        .to_string(),
                ),
            );
        }

        let was = self.in_teardown_or_after;
        if enters_after {
            self.in_teardown_or_after = true;
        }
        if let Some(receiver) = node.receiver() {
            self.visit(&receiver);
        }
        if let Some(arguments) = node.arguments() {
            for argument in arguments.arguments().iter() {
                self.visit(&argument);
            }
        }
        if let Some(block) = node.block() {
            self.visit(&block);
        }
        self.in_teardown_or_after = was;
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        // Also match `def teardown; ... travel_back; end`
        let is_teardown = node.name().as_slice() == b"teardown";

        let was = self.in_teardown_or_after;
        if is_teardown {
            self.in_teardown_or_after = true;
        }
        ruby_prism::visit_def_node(self, node);
        self.in_teardown_or_after = was;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::CopConfig;
    use std::collections::HashMap;

    crate::cop_rails_fixture_tests!(RedundantTravelBack, "cops/rails/redundant_travel_back", 5.2);

    #[test]
    fn skipped_when_railties_not_in_lockfile() {
        let source = b"RSpec.describe 'x' do\n  after do\n    travel_back\n  end\nend\n";
        let mut options = HashMap::new();
        options.insert(
            "TargetRailsVersion".to_string(),
            serde_yml::Value::Number(serde_yml::Number::from(5.2)),
        );
        let config = CopConfig {
            options,
            ..CopConfig::default()
        };

        let diagnostics = crate::testutil::run_cop_full_internal(
            &RedundantTravelBack,
            source,
            config,
            "spec/example_spec.rb",
        );
        assert!(
            diagnostics.is_empty(),
            "Should not fire when railties is not in Gemfile.lock (matches RuboCop's requires_gem gate)"
        );
    }
}
