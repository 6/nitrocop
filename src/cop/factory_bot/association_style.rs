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
            // explicit style: only process factory nodes (not trait nodes separately)
            if method_name != b"factory" {
                return;
            }

            // Collect all trait names defined in this factory (before moving body)
            let trait_names = collect_trait_names(&body);

            let children: Vec<_> = if let Some(stmts) = body.as_statements_node() {
                stmts.body().iter().collect()
            } else {
                vec![body]
            };

            // Check direct children and recurse into nested trait blocks
            check_implicit_in_children(self, source, &children, &trait_names, diagnostics);
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

    if call.block().is_some() {
        return false;
    }

    true
}

/// Check children for implicit associations, recursing into nested trait blocks.
fn check_implicit_in_children(
    cop: &AssociationStyle,
    source: &SourceFile,
    children: &[ruby_prism::Node<'_>],
    trait_names: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for child in children {
        // If this child is a `trait` call with a block, recurse into its body
        if let Some(call) = child.as_call_node() {
            if method_dispatch_predicates::is_command(&call, b"trait") {
                if let Some(block) = call.block() {
                    if let Some(block_node) = block.as_block_node() {
                        if let Some(body) = block_node.body() {
                            let nested: Vec<_> = if let Some(stmts) = body.as_statements_node() {
                                stmts.body().iter().collect()
                            } else {
                                vec![body]
                            };
                            check_implicit_in_children(
                                cop,
                                source,
                                &nested,
                                trait_names,
                                diagnostics,
                            );
                        }
                    }
                }
                continue;
            }
        }

        if is_implicit_association_with_traits(child, trait_names) {
            let loc = child.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(cop.diagnostic(
                source,
                line,
                column,
                "Use explicit style to define associations.".to_string(),
            ));
        }
    }
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
