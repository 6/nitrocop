use ruby_prism::Visit;

use crate::cop::factory_bot::FACTORY_BOT_DEFAULT_INCLUDE;
use crate::cop::shared::method_dispatch_predicates;
use crate::cop::shared::node_type::{
    ASSOC_NODE, BLOCK_NODE, CALL_NODE, HASH_NODE, KEYWORD_HASH_NODE, STATEMENTS_NODE, SYMBOL_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// ## Variant notes (EnforcedStyle: explicit)
///
/// For explicit style, trait-name calls inside nested `trait` blocks (e.g.
/// `with_ipv6` inside `trait :dualstack`) must not be flagged when the method
/// name matches a trait defined in the enclosing `factory`. Fixed by collecting
/// trait names from the factory body and passing them through when recursing
/// into nested trait blocks, rather than processing trait nodes independently.
///
/// Two additional fixes for explicit style:
/// 1. Block argument FN: `password_confirmation(&:password)` was not flagged
///    because `call.block().is_some()` returned true for `BlockArgumentNode`.
///    RuboCop's `implicit_association?` matcher uses `(send nil? ...)` which
///    only matches `send` nodes — in RuboCop AST, `foo do ... end` is a `block`
///    node (not `send`), but `foo(&:bar)` IS a `send` node. In Prism both are
///    CallNodes, so we distinguish by checking for `BlockNode` specifically.
/// 2. Nested factory FP: `premium` inside `factory :child` nested under
///    `factory :parent` was falsely flagged when `:premium` is a trait in the
///    parent. RuboCop's `trait_factory_node` finds the OUTERMOST enclosing
///    factory and collects trait names from it. Fixed by collecting trait names
///    from all enclosing factory blocks via parse tree traversal.
/// 3. Iterator-nested trait FN: traits inside iterator blocks (e.g.
///    `.each do |n| trait :"#{n}_type" do ... end end`) were missed because
///    the factory handler only recursed into direct trait children. Fixed by
///    processing both `factory` and `trait` nodes independently in check_node:
///    factory nodes check their non-trait children, and trait nodes check
///    their own children using enclosing factory trait names.
/// 4. Top-level trait FP: `trait` blocks defined at the `FactoryBot.define`
///    level (siblings of factory blocks, not nested inside any factory) were
///    falsely flagging their children as implicit associations. RuboCop's
///    `trait_factory_node` returns nil for such traits, so they are never
///    processed. Fixed by checking `has_enclosing_factory` before processing
///    trait blocks.
pub struct AssociationStyle;

/// Ruby keywords that cannot be implicit associations.
const RUBY_KEYWORDS: &[&str] = &[
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "defined?",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "false",
    "for",
    "if",
    "in",
    "module",
    "next",
    "nil",
    "not",
    "or",
    "redo",
    "rescue",
    "retry",
    "return",
    "self",
    "super",
    "then",
    "true",
    "undef",
    "unless",
    "until",
    "when",
    "while",
    "yield",
    "__FILE__",
    "__LINE__",
    "__ENCODING__",
];

/// FactoryBot reserved methods that should not be treated as implicit associations.
const RESERVED_METHODS: &[&str] = &[
    "add_attribute",
    "after",
    "association",
    "before",
    "callback",
    "ignore",
    "initialize_with",
    "sequence",
    "skip_create",
    "to_create",
    "__send__",
    "__id__",
    "nil?",
    "send",
    "object_id",
    "extend",
    "instance_eval",
    "initialize",
    "block_given?",
    "raise",
    "caller",
    "method",
    "factory",
    "trait",
    "traits_for_enum",
    "transient",
];

fn is_reserved_method(name: &str) -> bool {
    RESERVED_METHODS.contains(&name)
}

fn is_ruby_keyword(name: &str) -> bool {
    RUBY_KEYWORDS.contains(&name)
}

impl Cop for AssociationStyle {
    fn name(&self) -> &'static str {
        "FactoryBot/AssociationStyle"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        FACTORY_BOT_DEFAULT_INCLUDE
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ASSOC_NODE,
            BLOCK_NODE,
            CALL_NODE,
            HASH_NODE,
            KEYWORD_HASH_NODE,
            STATEMENTS_NODE,
            SYMBOL_NODE,
        ]
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
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // Only trigger on `factory` or `trait` calls
        let method_name = call.name().as_slice();
        if method_name != b"factory" && method_name != b"trait" {
            return;
        }

        // Must have no receiver (bare `factory` / `trait`)
        if call.receiver().is_some() {
            return;
        }

        // Must have a block
        let block = match call.block() {
            Some(b) => b,
            None => return,
        };

        let block_node = match block.as_block_node() {
            Some(b) => b,
            None => return,
        };

        let body = match block_node.body() {
            Some(body) => body,
            None => return,
        };

        let style = config.get_str("EnforcedStyle", "implicit");

        if style == "explicit" {
            let my_start = call.location().start_offset();
            let my_end = call.location().end_offset();

            // RuboCop's trait_factory_node returns nil for traits at the
            // FactoryBot.define level (not inside any factory block), so they
            // are never flagged. We replicate this: skip trait blocks that
            // have no enclosing factory.
            if method_name == b"trait" && !has_enclosing_factory(_parse_result, my_start, my_end) {
                return;
            }

            // Collect trait names before extracting children (body may be moved)
            let factory_trait_names = if method_name == b"factory" {
                let mut names = collect_trait_names(&body);
                let enclosing =
                    collect_enclosing_factory_trait_names(_parse_result, my_start, my_end);
                names.extend(enclosing);
                names
            } else {
                collect_enclosing_factory_trait_names(_parse_result, my_start, my_end)
            };

            let children: Vec<_> = if let Some(stmts) = body.as_statements_node() {
                stmts.body().iter().collect()
            } else {
                vec![body]
            };

            if method_name == b"factory" {
                // Check direct non-trait children only; trait blocks are processed
                // independently when check_node fires for the trait CallNode.
                for child in &children {
                    if let Some(c) = child.as_call_node() {
                        if method_dispatch_predicates::is_command(&c, b"trait") {
                            continue;
                        }
                    }
                    if is_implicit_association_with_traits(child, &factory_trait_names) {
                        let loc = child.location();
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Use explicit style to define associations.".to_string(),
                        ));
                    }
                }
            } else {
                // trait node: check its children for implicit associations.
                // This handles traits nested inside iterator blocks (e.g. .each do)
                // that the factory handler can't reach via direct child traversal.
                for child in &children {
                    if is_implicit_association_with_traits(child, &factory_trait_names) {
                        let loc = child.location();
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Use explicit style to define associations.".to_string(),
                        ));
                    }
                }
            }
        } else {
            let children: Vec<_> = if let Some(stmts) = body.as_statements_node() {
                stmts.body().iter().collect()
            } else {
                vec![body]
            };

            for child in &children {
                if is_explicit_association(child)
                    && !has_strategy_build(child)
                    && !has_keyword_arg(child)
                {
                    let loc = child.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use implicit style to define associations.".to_string(),
                    ));
                }
            }
        }
    }
}

/// Check if a node is an explicit `association :name` call.
fn is_explicit_association(node: &ruby_prism::Node<'_>) -> bool {
    let call = match node.as_call_node() {
        Some(c) => c,
        None => return false,
    };

    if call.receiver().is_some() {
        return false;
    }

    if call.name().as_slice() != b"association" {
        return false;
    }

    let args = match call.arguments() {
        Some(a) => a,
        None => return false,
    };

    let arg_list: Vec<_> = args.arguments().iter().collect();
    if arg_list.is_empty() {
        return false;
    }

    // First argument must be a symbol
    arg_list[0].as_symbol_node().is_some()
}

/// Check if an explicit association has `strategy: :build`.
fn has_strategy_build(node: &ruby_prism::Node<'_>) -> bool {
    let call = match node.as_call_node() {
        Some(c) => c,
        None => return false,
    };

    let args = match call.arguments() {
        Some(a) => a,
        None => return false,
    };

    for arg in args.arguments().iter() {
        if let Some(hash) = arg.as_keyword_hash_node() {
            for elem in hash.elements().iter() {
                if let Some(pair) = elem.as_assoc_node() {
                    if let Some(key_sym) = pair.key().as_symbol_node() {
                        if key_sym.unescaped() == b"strategy" {
                            if let Some(val_sym) = pair.value().as_symbol_node() {
                                if val_sym.unescaped() == b"build" {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(hash) = arg.as_hash_node() {
            for elem in hash.elements().iter() {
                if let Some(pair) = elem.as_assoc_node() {
                    if let Some(key_sym) = pair.key().as_symbol_node() {
                        if key_sym.unescaped() == b"strategy" {
                            if let Some(val_sym) = pair.value().as_symbol_node() {
                                if val_sym.unescaped() == b"build" {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

/// Check if an explicit association has a Ruby keyword as association name argument.
fn has_keyword_arg(node: &ruby_prism::Node<'_>) -> bool {
    let call = match node.as_call_node() {
        Some(c) => c,
        None => return false,
    };

    let args = match call.arguments() {
        Some(a) => a,
        None => return false,
    };

    for arg in args.arguments().iter() {
        if let Some(sym) = arg.as_symbol_node() {
            let name = std::str::from_utf8(sym.unescaped()).unwrap_or("");
            if is_ruby_keyword(name) {
                return true;
            }
        }
    }
    false
}

/// Check whether the given source range is enclosed by any `factory` block.
/// RuboCop's `trait_factory_node` returns nil for traits defined at the
/// `FactoryBot.define` level (not nested inside any factory), causing them
/// to be skipped. We replicate this: if a `trait` is not inside any factory,
/// its children should NOT be flagged.
fn has_enclosing_factory(
    parse_result: &ruby_prism::ParseResult<'_>,
    my_start: usize,
    my_end: usize,
) -> bool {
    struct FactoryEnclosureChecker {
        my_start: usize,
        my_end: usize,
        found: bool,
    }

    impl<'pr> Visit<'pr> for FactoryEnclosureChecker {
        fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
            if self.found {
                return;
            }
            if method_dispatch_predicates::is_command(node, b"factory") {
                let start = node.location().start_offset();
                let end = node.location().end_offset();
                if start < self.my_start && end > self.my_end {
                    self.found = true;
                    return;
                }
            }
            ruby_prism::visit_call_node(self, node);
        }
    }

    let mut checker = FactoryEnclosureChecker {
        my_start,
        my_end,
        found: false,
    };
    checker.visit(&parse_result.node());
    checker.found
}

/// Collect trait names from all factory blocks that enclose the given source range.
/// This replicates RuboCop's trait_factory_node behavior, which finds the outermost
/// enclosing factory and collects trait names from it.
fn collect_enclosing_factory_trait_names(
    parse_result: &ruby_prism::ParseResult<'_>,
    my_start: usize,
    my_end: usize,
) -> Vec<String> {
    struct EnclosingFactoryFinder {
        my_start: usize,
        my_end: usize,
        trait_names: Vec<String>,
    }

    impl<'pr> Visit<'pr> for EnclosingFactoryFinder {
        fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
            if method_dispatch_predicates::is_command(node, b"factory") {
                let start = node.location().start_offset();
                let end = node.location().end_offset();
                // If this factory strictly CONTAINS our factory
                if start < self.my_start && end > self.my_end {
                    if let Some(block) = node.block() {
                        if let Some(block_node) = block.as_block_node() {
                            if let Some(body) = block_node.body() {
                                let names = collect_trait_names(&body);
                                self.trait_names.extend(names);
                            }
                        }
                    }
                }
            }
            ruby_prism::visit_call_node(self, node);
        }
    }

    let mut finder = EnclosingFactoryFinder {
        my_start,
        my_end,
        trait_names: Vec::new(),
    };
    finder.visit(&parse_result.node());
    finder.trait_names
}

/// Collect all trait names defined within a factory body (recursively).
fn collect_trait_names(body: &ruby_prism::Node<'_>) -> Vec<String> {
    struct TraitCollector {
        trait_names: Vec<String>,
    }
    impl<'pr> Visit<'pr> for TraitCollector {
        fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
            if method_dispatch_predicates::is_command(node, b"trait") {
                if let Some(args) = node.arguments() {
                    let arg_list: Vec<_> = args.arguments().iter().collect();
                    if let Some(sym) = arg_list.first().and_then(|a| a.as_symbol_node()) {
                        if let Ok(name) = std::str::from_utf8(sym.unescaped()) {
                            self.trait_names.push(name.to_string());
                        }
                    }
                }
            }
            ruby_prism::visit_call_node(self, node);
        }
    }

    let mut collector = TraitCollector {
        trait_names: Vec::new(),
    };
    collector.visit(body);
    collector.trait_names
}

/// Check if a node is an implicit association in explicit style,
/// using a pre-collected list of trait names from the enclosing factory.
fn is_implicit_association_with_traits(
    node: &ruby_prism::Node<'_>,
    trait_names: &[String],
) -> bool {
    let call = match node.as_call_node() {
        Some(c) => c,
        None => return false,
    };

    if call.receiver().is_some() {
        return false;
    }

    let method_name = std::str::from_utf8(call.name().as_slice()).unwrap_or("");

    if is_reserved_method(method_name) {
        return false;
    }

    if trait_names.iter().any(|n| n == method_name) {
        return false;
    }

    // Only exclude calls with do/end or {} blocks (BlockNode), not block
    // arguments like &:password (BlockArgumentNode). RuboCop's implicit_association?
    // matcher only matches `send` nodes; in RuboCop AST, `foo do ... end` is a
    // `block` node (not a `send`), so it never matches. In Prism, both are CallNodes
    // but we can distinguish via block type.
    if let Some(block) = call.block() {
        if block.as_block_node().is_some() {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(AssociationStyle, "cops/factorybot/association_style");

    fn explicit_config() -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("explicit".to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn offense_explicit() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &AssociationStyle,
            include_bytes!(
                "../../../tests/fixtures/cops/factorybot/association_style/offense.explicit.rb"
            ),
            explicit_config(),
        );
    }

    #[test]
    fn no_offense_explicit() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &AssociationStyle,
            include_bytes!(
                "../../../tests/fixtures/cops/factorybot/association_style/no_offense.explicit.rb"
            ),
            explicit_config(),
        );
    }
}
