use crate::cop::shared::method_dispatch_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Rails/HttpPositionalArguments cop.
///
/// Detects HTTP request method calls (get, post, put, patch, delete, head) that use
/// positional hash arguments instead of proper keyword arguments.
///
/// ## Detection Patterns
///
/// This cop detects two forms of old-style positional arguments:
///
/// 1. **HashNode with string keys**: `get :index, { "ACCEPT" => "text/html" }`
///    - The old-style `{ key => value }` hash format uses string keys
///    - Flagged because it uses positional args instead of `headers: { ... }`
///
/// 2. **KeywordHashNode with hash rockets**: `get :show, :id => 12`
///    - Hash rocket syntax (`:key => value`) is old style
///    - Proper keyword args use label syntax: `user_id: 1` (no `=>` operator)
///    - The distinguishing factor is the `operator_loc` on the AssocNode:
///      - Hash rockets have an `operator_loc` pointing to `=>`
///      - Proper keyword args have no operator_loc (implicit)
pub struct HttpPositionalArguments;

const HTTP_METHODS: &[&[u8]] = &[b"get", b"post", b"put", b"patch", b"delete", b"head"];

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
                if arg_list.len() >= 2 {
                    // Check if any arg (after first, which is the path/action) is a hash
                    // that is NOT using proper keyword argument style
                    let mut found_offense = false;
                    for arg in arg_list.iter().skip(1) {
                        // HashNode: explicit brace hash like `{ user_id: 1 }` or `{ "key" => val }`
                        // Need to check if it's keyword syntax (symbol keys) or old rocket style
                        if let Some(hash) = arg.as_hash_node() {
                            // Flag if this hash does NOT use proper keyword syntax
                            if !is_keyword_syntax(&hash) {
                                found_offense = true;
                            }
                        }
                        // KeywordHashNode: keyword-arg-style hash like `:id => 12` or `user_id: 1`
                        // `:id => 12` is old hash rocket style (offense)
                        // `user_id: 1` is proper keyword style (no offense)
                        if let Some(kw_hash) = arg.as_keyword_hash_node() {
                            if !is_proper_keyword_args(&kw_hash) {
                                found_offense = true;
                            }
                        }
                    }
                    if found_offense {
                        let loc = node.location();
                        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                        self.diagnostics.push(self.cop.diagnostic(
                            self.source,
                            line,
                            column,
                            format!("Use keyword arguments instead of positional arguments for http call: `{}`.", String::from_utf8_lossy(method_name)),
                        ));
                    }
                }
            }
        }
        ruby_prism::visit_call_node(self, node);
    }
}

/// Check if a HashNode uses proper keyword argument style (symbol keys like `user_id: 1`)
/// vs old hash rocket style (`"key" => value` or `:key => value`)
/// Also checks if the keys are special keyword args recognized by Rails HTTP request methods
fn is_keyword_syntax(hash: &ruby_prism::HashNode<'_>) -> bool {
    const KEYWORD_ARGS: &[&[u8]] = &[
        b"method", b"params", b"session", b"body", b"flash", b"xhr", b"as", b"headers", b"env",
        b"to",
    ];

    hash.elements().iter().any(|elem| {
        if let Some(assoc) = elem.as_assoc_node() {
            let key = assoc.key();
            // Check if key is a symbol with one of the special keyword arg names
            if let Some(sym) = key.as_symbol_node() {
                if KEYWORD_ARGS.contains(&sym.unescaped()) {
                    return true;
                }
            }
            // Proper keyword args have symbol keys (user_id: 1 style)
            if key.as_symbol_node().is_some() || key.as_interpolated_symbol_node().is_some() {
                return true;
            }
        }
        false
    })
}

/// Check if a KeywordHashNode uses proper keyword args style (`user_id: 1`)
/// Hash rocket style (`:id => 12`) is NOT proper keyword args
fn is_proper_keyword_args(kw_hash: &ruby_prism::KeywordHashNode<'_>) -> bool {
    let elems = kw_hash.elements();
    for elem in elems.iter() {
        if let Some(assoc) = elem.as_assoc_node() {
            let key = assoc.key();
            if key.as_symbol_node().is_some() {
                let op_loc = assoc.operator_loc();
                if op_loc.is_some() {
                    return false;
                }
            }
        }
    }
    true
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
