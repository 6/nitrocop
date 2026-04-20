use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Matches RuboCop's `Rails/RedundantTravelBack` for Rails 5.2+ test files.
///
/// The remaining corpus false positives came from two RuboCop quirks:
///
/// - The cop's default `Include` is exactly `spec/**/*.rb` and `test/**/*.rb`.
///   Broadening that to `**/spec/**/*.rb` wrongly linted nested engine/module
///   specs such as `modules/**/spec/**`.
/// - `minimum_target_rails_version 5.2` also depends on the actual `railties`
///   gem version. A corpus overlay can force `TargetRailsVersion: 7.0`, but
///   Rails 4.2 repos must still be skipped.
///
/// It also intentionally matches any block method named `after`, including
/// receiver forms such as `config.after`, because that is what RuboCop does.
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
        &["spec/**/*.rb", "test/**/*.rb"]
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
        if config
            .railties_version()
            .is_some_and(|version| version < 5.2)
        {
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
    use crate::cli::Args;
    use crate::cop::CopConfig;
    use crate::cop::autocorrect_allowlist::AutocorrectAllowlist;
    use crate::cop::registry::CopRegistry;
    use crate::cop::tiers::TierMap;
    use crate::parse::source::SourceFile;
    use clap::Parser;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    crate::cop_rails_fixture_tests!(RedundantTravelBack, "cops/rails/redundant_travel_back", 5.2);

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nitrocop_test_redundant_travel_back_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(root: &Path, rel: &str, contents: &[u8]) -> PathBuf {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, contents).unwrap();
        path
    }

    fn args_for_repo(repo: &Path) -> Args {
        Args::parse_from([
            "nitrocop",
            repo.to_str().unwrap(),
            "--preview",
            "--no-cache",
            "--only",
            "Rails/RedundantTravelBack",
        ])
    }

    fn lint_repo_file(repo: &Path, file_path: &Path) -> crate::linter::LintResult {
        let config = crate::config::load_config(None, Some(repo), None).unwrap();
        let registry = CopRegistry::default_registry();
        let tier_map = TierMap::load();
        let args = args_for_repo(repo);
        let allowlist = AutocorrectAllowlist::load();
        let source = SourceFile::from_path(file_path).unwrap();

        crate::linter::lint_source(&source, &config, &registry, &args, &tier_map, &allowlist)
    }

    #[test]
    fn flags_top_level_spec_files() {
        let dir = temp_dir("top_level_spec");
        write_file(
            &dir,
            ".rubocop.yml",
            b"AllCops:\n  TargetRailsVersion: 7.0\nRails/RedundantTravelBack:\n  Enabled: true\n",
        );
        write_file(
            &dir,
            "Gemfile.lock",
            b"GEM\n  specs:\n    railties (7.0.0)\n",
        );
        let file = write_file(
            &dir,
            "spec/example_spec.rb",
            b"after do\n  travel_back\nend\n",
        );

        let result = lint_repo_file(&dir, &file);

        assert_eq!(
            result.diagnostics.len(),
            1,
            "diagnostics: {:?}",
            result.diagnostics
        );
        assert_eq!(result.diagnostics[0].cop_name, "Rails/RedundantTravelBack");
        assert_eq!(result.diagnostics[0].location.line, 2);
    }

    #[test]
    fn skips_nested_module_spec_files() {
        let dir = temp_dir("nested_module_spec");
        write_file(
            &dir,
            ".rubocop.yml",
            b"AllCops:\n  TargetRailsVersion: 7.0\nRails/RedundantTravelBack:\n  Enabled: true\n",
        );
        write_file(
            &dir,
            "Gemfile.lock",
            b"GEM\n  specs:\n    railties (7.0.0)\n",
        );
        let file = write_file(
            &dir,
            "modules/foo/spec/example_spec.rb",
            b"after do\n  travel_back\nend\n",
        );

        let result = lint_repo_file(&dir, &file);

        assert!(
            result.diagnostics.is_empty(),
            "nested modules/**/spec/** path should not match RuboCop's default Include: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn skipped_when_actual_railties_version_is_below_minimum() {
        let dir = temp_dir("old_railties");
        write_file(
            &dir,
            ".rubocop.yml",
            b"AllCops:\n  TargetRailsVersion: 7.0\nRails/RedundantTravelBack:\n  Enabled: true\n",
        );
        write_file(
            &dir,
            "Gemfile.lock",
            b"GEM\n  specs:\n    railties (4.2.3)\n",
        );
        let file = write_file(
            &dir,
            "spec/example_spec.rb",
            b"after do\n  travel_back\nend\n",
        );

        let result = lint_repo_file(&dir, &file);

        assert!(
            result.diagnostics.is_empty(),
            "Should not fire when actual railties version is below 5.2: {:?}",
            result.diagnostics
        );
    }

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
