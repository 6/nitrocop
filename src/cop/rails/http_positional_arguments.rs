use crate::cop::shared::method_dispatch_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

pub struct HttpPositionalArguments;

const HTTP_METHODS: &[&[u8]] = &[b"get", b"post", b"put", b"patch", b"delete", b"head"];

impl Cop for HttpPositionalArguments {
    fn name(&self) -> &'static str {
        "Rails/HttpPositionalArguments"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // minimum_target_rails_version 5.0
        if !config.rails_version_at_least(5.0) {
            return;
        }

        // First, check if the file includes Rack::Test::Methods — if so, skip entirely
        let mut checker = RackTestChecker { found: false };
        checker.visit(&parse_result.node());
        if checker.found {
            return;
        }

        let mut visitor = HttpPosArgsVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

/// Scans AST for `include Rack::Test::Methods`
struct RackTestChecker {
    found: bool,
}

impl<'pr> Visit<'pr> for RackTestChecker {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if !self.found && method_dispatch_predicates::is_command(node, b"include") {
            if let Some(args) = node.arguments() {
                for arg in args.arguments().iter() {
                    if is_rack_test_methods(&arg) {
                        self.found = true;
                        return;
                    }
                }
            }
        }
        if !self.found {
            ruby_prism::visit_call_node(self, node);
        }
    }
}

/// Check if node is `Rack::Test::Methods` constant path
fn is_rack_test_methods(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(cp) = node.as_constant_path_node() {
        // Check Methods
        if cp.name().is_none_or(|n| n.as_slice() != b"Methods") {
            return false;
        }
        // Check parent is Rack::Test
        if let Some(parent) = cp.parent() {
            if let Some(cp2) = parent.as_constant_path_node() {
                if cp2.name().is_none_or(|n| n.as_slice() != b"Test") {
                    return false;
                }
                // Check grandparent is Rack
                if let Some(gp) = cp2.parent() {
                    if let Some(cr) = gp.as_constant_read_node() {
                        return cr.name().as_slice() == b"Rack";
                    }
                }
            }
        }
    }
    false
}

struct HttpPosArgsVisitor<'a> {
    cop: &'a HttpPositionalArguments,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
}

impl<'pr> Visit<'pr> for HttpPosArgsVisitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let method_name = node.name().as_slice();
        if HTTP_METHODS.contains(&method_name) && node.receiver().is_none() {
            if let Some(args) = node.arguments() {
                let arg_list: Vec<_> = args.arguments().iter().collect();
                // Only flag explicit HashNode (old-style positional: `get path, {params}, headers`).
                // A keyword_hash_node means keyword args (`get path, params: ...`), which is
                // the correct new-style syntax this cop promotes — don't flag it.
                if arg_list.len() >= 3 && arg_list[1].as_hash_node().is_some() {
                    let loc = node.location();
                    let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                    self.diagnostics.push(self.cop.diagnostic(
                        self.source,
                        line,
                        column,
                        "Use keyword arguments for HTTP request methods.".to_string(),
                    ));
                }
            }
        }
        ruby_prism::visit_call_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::CopConfig;
    use std::collections::HashMap;

    fn config_with_rails(version: f64) -> CopConfig {
        let mut options = HashMap::new();
        options.insert(
            "TargetRailsVersion".to_string(),
            serde_yml::Value::Number(serde_yml::value::Number::from(version)),
        );
        options.insert(
            "__RailtiesInLockfile".to_string(),
            serde_yml::Value::Bool(true),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    #[test]
    fn offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &HttpPositionalArguments,
            include_bytes!(
                "../../../tests/fixtures/cops/rails/http_positional_arguments/offense.rb"
            ),
            config_with_rails(5.0),
        );
    }

    #[test]
    fn no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HttpPositionalArguments,
            include_bytes!(
                "../../../tests/fixtures/cops/rails/http_positional_arguments/no_offense.rb"
            ),
            config_with_rails(5.0),
        );
    }

    #[test]
    fn skipped_when_no_target_rails_version() {
        // Non-Rails projects (e.g. sinatra) have no TargetRailsVersion.
        // RuboCop uses `requires_gem('railties', '>= 5.0')` which skips the cop
        // entirely when railties is not installed. Nitrocop should do the same.
        let source = b"get :index, { user_id: 1 }, { \"ACCEPT\" => \"text/html\" }\n";
        let diagnostics = crate::testutil::run_cop_full_internal(
            &HttpPositionalArguments,
            source,
            CopConfig::default(), // no TargetRailsVersion
            "test/some_test.rb",
        );
        assert!(
            diagnostics.is_empty(),
            "Should not fire when TargetRailsVersion is not set (non-Rails project)"
        );
    }
}
