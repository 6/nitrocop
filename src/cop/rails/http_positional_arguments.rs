use crate::cop::shared::method_dispatch_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Mirrors RuboCop's first positional data-argument check for Rails HTTP helpers.
///
/// Prism splits old request data across `HashNode`, `KeywordHashNode`,
/// `ForwardingArgumentsNode`, and kw-splat shapes. RuboCop also flags plain
/// positional values like `post :create, confirmation_data`, while skipping
/// routing DSL blocks, files using `Rack::Test::Methods`, lone `format:`,
/// special keyword args like `params:` / `headers:`, kw-splats, and forwarded
/// args / kwargs.
///
/// RuboCop only uses `minimum_target_rails_version 5.0` here; it does not gate
/// the cop behind `requires_gem 'railties'`. So `TargetRailsVersion` alone must
/// enable the cop, even when no lockfile metadata is present.
pub struct HttpPositionalArguments;

const HTTP_METHODS: &[&[u8]] = &[b"get", b"post", b"put", b"patch", b"delete", b"head"];
const ROUTING_METHODS: &[&[u8]] = &[b"draw", b"routes"];
const KEYWORD_ARGS: &[&[u8]] = &[
    b"method", b"params", b"session", b"body", b"flash", b"xhr", b"as", b"headers", b"env", b"to",
];

fn hash_elements<'pr>(node: &'pr ruby_prism::Node<'pr>) -> Option<Vec<ruby_prism::Node<'pr>>> {
    if let Some(hash) = node.as_hash_node() {
        Some(hash.elements().iter().collect())
    } else {
        node.as_keyword_hash_node()
            .map(|hash| hash.elements().iter().collect())
    }
}

fn hash_pairs<'pr>(node: &'pr ruby_prism::Node<'pr>) -> Option<Vec<ruby_prism::AssocNode<'pr>>> {
    let elements = hash_elements(node)?;

    let pairs: Vec<ruby_prism::AssocNode<'pr>> = elements
        .into_iter()
        .filter_map(|elem: ruby_prism::Node<'pr>| elem.as_assoc_node())
        .collect();
    Some(pairs)
}

fn special_keyword_arg(node: &ruby_prism::Node<'_>) -> bool {
    node.as_symbol_node()
        .is_some_and(|sym| KEYWORD_ARGS.contains(&sym.unescaped()))
}

fn format_arg(node: &ruby_prism::Node<'_>) -> bool {
    node.as_symbol_node()
        .is_some_and(|sym| sym.unescaped() == b"format")
}

fn is_kwsplat_hash<'pr>(node: &'pr ruby_prism::Node<'pr>) -> bool {
    let Some(elements) = hash_elements(node) else {
        return false;
    };

    elements.len() == 1
        && elements[0].as_assoc_splat_node().is_some_and(
            |assoc_splat: ruby_prism::AssocSplatNode<'pr>| assoc_splat.value().is_some(),
        )
}

fn is_forwarded_kwrestarg<'pr>(node: &'pr ruby_prism::Node<'pr>) -> bool {
    let Some(elements) = hash_elements(node) else {
        return false;
    };

    elements.len() == 1
        && elements[0].as_assoc_splat_node().is_some_and(
            |assoc_splat: ruby_prism::AssocSplatNode<'pr>| assoc_splat.value().is_none(),
        )
}

fn needs_conversion<'pr>(node: &'pr ruby_prism::Node<'pr>) -> bool {
    if node.as_forwarding_arguments_node().is_some() || is_forwarded_kwrestarg(node) {
        return false;
    }

    let Some(pairs) = hash_pairs(node) else {
        return true;
    };

    if is_kwsplat_hash(node) {
        return false;
    }

    pairs.iter().all(|pair: &ruby_prism::AssocNode<'pr>| {
        !(special_keyword_arg(&pair.key()) || format_arg(&pair.key()) && pairs.len() == 1)
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
        // This cop does NOT use `requires_gem 'railties'`, so do not require
        // `__RailtiesInLockfile` here.
        if !config.target_rails_version().is_some_and(|v| v >= 5.0) {
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
            ancestors: Vec::new(),
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
        if cp.name().is_none_or(|n| n.as_slice() != b"Methods") {
            return false;
        }
        if let Some(parent) = cp.parent() {
            if let Some(cp2) = parent.as_constant_path_node() {
                if cp2.name().is_none_or(|n| n.as_slice() != b"Test") {
                    return false;
                }
                if let Some(gp) = cp2.parent() {
                    if let Some(cr) = gp.as_constant_read_node() {
                        return cr.name().as_slice() == b"Rack";
                    }
                    if let Some(cp3) = gp.as_constant_path_node() {
                        return cp3.parent().is_none()
                            && cp3.name().is_some_and(|n| n.as_slice() == b"Rack");
                    }
                }
            }
        }
    }
    false
}

struct HttpPosArgsVisitor<'a, 'pr> {
    cop: &'a HttpPositionalArguments,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    ancestors: Vec<ruby_prism::Node<'pr>>,
}

impl<'a, 'pr> HttpPosArgsVisitor<'a, 'pr> {
    fn in_routing_block(&self) -> bool {
        self.ancestors
            .iter()
            .enumerate()
            .take(self.ancestors.len().saturating_sub(1))
            .any(|(idx, node)| {
                node.as_block_node().is_some()
                    && idx > 0
                    && self.ancestors[idx - 1]
                        .as_call_node()
                        .is_some_and(|call| ROUTING_METHODS.contains(&call.name().as_slice()))
            })
    }
}

impl<'a, 'pr> Visit<'pr> for HttpPosArgsVisitor<'a, 'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {}

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let method_name = node.name().as_slice();
        if HTTP_METHODS.contains(&method_name)
            && node.receiver().is_none()
            && !self.in_routing_block()
        {
            if let Some(args) = node.arguments() {
                let arg_list: Vec<_> = args.arguments().iter().collect();
                if arg_list.len() >= 2 && needs_conversion(&arg_list[1]) {
                    let loc = arg_list[1].location();
                    let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                    self.diagnostics.push(self.cop.diagnostic(
                        self.source,
                        line,
                        column,
                        format!(
                            "Use keyword arguments instead of positional arguments for http call: `{}`.",
                            std::str::from_utf8(method_name).unwrap_or("get")
                        ),
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
    fn skipped_with_rack_test_methods_include() {
        let source = b"include Rack::Test::Methods\n\nget :create, user_id: @user.id\n";
        let diagnostics = crate::testutil::run_cop_full_internal(
            &HttpPositionalArguments,
            source,
            config_with_rails(5.0),
            "spec/some_spec.rb",
        );
        assert!(
            diagnostics.is_empty(),
            "Should not fire when Rack::Test::Methods is included"
        );
    }

    #[test]
    fn skipped_with_cbase_rack_test_methods_include() {
        let source = b"include ::Rack::Test::Methods\n\nget :create, user_id: @user.id\n";
        let diagnostics = crate::testutil::run_cop_full_internal(
            &HttpPositionalArguments,
            source,
            config_with_rails(5.0),
            "spec/some_spec.rb",
        );
        assert!(
            diagnostics.is_empty(),
            "Should not fire when ::Rack::Test::Methods is included"
        );
    }

    #[test]
    fn skipped_when_no_target_rails_version() {
        // Non-Rails projects (e.g. sinatra) have no TargetRailsVersion.
        // RuboCop's `minimum_target_rails_version 5.0` skips when no target
        // version is configured.
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

    #[test]
    fn fires_without_railties_in_lockfile_when_target_rails_version_is_set() {
        let mut options = HashMap::new();
        options.insert(
            "TargetRailsVersion".to_string(),
            serde_yml::Value::Number(serde_yml::value::Number::from(5.0)),
        );
        let config = CopConfig {
            options,
            ..CopConfig::default()
        };
        let diagnostics = crate::testutil::run_cop_full_internal(
            &HttpPositionalArguments,
            b"get :show, :id => 12\n",
            config,
            "spec/some_spec.rb",
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "Should fire with TargetRailsVersion even without railties in lockfile"
        );
    }
}
