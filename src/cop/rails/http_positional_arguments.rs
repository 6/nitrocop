use crate::cop::shared::method_dispatch_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Detects uses of HTTP request methods (get, post, put, patch, delete, head)
/// with positional hash arguments that should use keyword arguments.
///
/// Originally this cop only detected 3+ argument forms like `get :index, {params}, {headers}`
/// where the second argument was an explicit HashNode. It missed 2-argument forms like
/// `get :edit, :id => 12` (hash rocket syntax) which Prism parses as KeywordHashNode.
///
/// The fix expands detection to any call with 2+ arguments where the second argument
/// is a HashNode or KeywordHashNode, provided it doesn't contain special keyword args
/// (params:, session:, headers:, etc.). Uses RuboCop's message format including the
/// HTTP verb for consistency.
pub struct HttpPositionalArguments;

const HTTP_METHODS: &[&[u8]] = &[b"get", b"post", b"put", b"patch", b"delete", b"head"];

/// Keys that are valid keyword args for HTTP request methods and should not be flagged.
const SPECIAL_KEYWORD_ARGS: &[&[u8]] = &[
    b"method", b"params", b"session", b"body", b"flash", b"xhr", b"as", b"headers", b"env", b"to",
];

/// Check if a hash or keyword hash has a special keyword arg as a key.
fn has_special_keyword_arg(node: &ruby_prism::Node<'_>) -> bool {
    let elements = if let Some(hash) = node.as_hash_node() {
        hash.elements()
    } else if let Some(kw_hash) = node.as_keyword_hash_node() {
        kw_hash.elements()
    } else {
        return false;
    };

    elements.iter().any(|el| {
        if let Some(assoc) = el.as_assoc_node() {
            if let Some(key) = assoc.key().as_symbol_node() {
                SPECIAL_KEYWORD_ARGS.contains(&key.unescaped())
            } else {
                false
            }
        } else {
            false
        }
    })
}

impl Cop for HttpPositionalArguments {
    fn name(&self) -> &'static str {
        "Rails/HttpPositionalArguments"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["spec/**/*", "test/**/*"]
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
                // Flag old-style positional hash args:
                // - `get path, {params}, headers` (explicit HashNode braces)
                // - `get path, :id => 12` (hash rocket syntax, may be KeywordHashNode)
                // Don't flag if the hash contains special keyword args like params:, session:, etc.
                if arg_list.len() >= 2 {
                    let second_arg = &arg_list[1];
                    let is_hash = second_arg.as_hash_node().is_some()
                        || second_arg.as_keyword_hash_node().is_some();
                    if is_hash && !has_special_keyword_arg(second_arg) {
                        let loc = node.location();
                        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                        let verb = std::str::from_utf8(method_name).unwrap_or("http");
                        let msg = format!(
                            "Use keyword arguments instead of positional arguments for http call: `{}`.",
                            verb
                        );
                        self.diagnostics
                            .push(self.cop.diagnostic(self.source, line, column, msg));
                    }
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
