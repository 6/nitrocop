use crate::cop::shared::node_type::{AND_NODE, OR_NODE, UNLESS_NODE};
use crate::cop::shared::predicate_operator_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Matches RuboCop's mixed-logical-operator checks for `unless` predicates.
///
/// Fixed FNs where Prism hid nested `&&`/`||` inside wrapper nodes that the old
/// recursion skipped, including assignment values, unary `!` calls, method
/// arguments, and attached blocks. We still preserve RuboCop's top-level paren
/// behavior by only treating the predicate as direct `and`/`or` when the root
/// node itself is an `AndNode` or `OrNode`.
///
/// Fixed 11 FP / 11 FN pairs caused by two issues:
/// 1. Location: `keyword_loc()` → `location()`. For modifier `unless` spanning
///    multiple lines, RuboCop's `add_offense(node)` uses `node.source_range`
///    which starts at the statement beginning, not the `unless` keyword.
/// 2. Messages: RuboCop uses `"in an \`unless\`."` not `"in \`unless\` conditions."`.
///
/// Variant FP fix (`EnforcedStyle: forbid_logical_operators`): Prism exposes
/// pattern-matching guards like `in pattern unless a || b` as `UnlessNode`s
/// inside the `in` clause, but RuboCop's `on_if` callback only sees real
/// `if`/`unless` statements. We skip guard nodes whose line prefix is just
/// `in`, while still flagging ordinary `work unless a || b`.
pub struct UnlessLogicalOperators;

impl Cop for UnlessLogicalOperators {
    fn name(&self) -> &'static str {
        "Style/UnlessLogicalOperators"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[AND_NODE, OR_NODE, UNLESS_NODE]
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
        let enforced_style = config.get_str("EnforcedStyle", "forbid_mixed_logical_operators");

        let unless_node = match node.as_unless_node() {
            Some(u) => u,
            None => return,
        };

        if is_pattern_matching_guard(source, node) {
            return;
        }

        let predicate = unless_node.predicate();

        match enforced_style {
            "forbid_logical_operators" => {
                // Flag any logical operators in unless conditions
                if contains_logical_operator(&predicate) {
                    let (line, column) =
                        source.offset_to_line_col(unless_node.location().start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Do not use any logical operator in an `unless`.".to_string(),
                    ));
                }
            }
            _ => {
                // Flag mixed logical operators (both && and ||)
                if contains_mixed_logical_operators(&predicate) {
                    let (line, column) =
                        source.offset_to_line_col(unless_node.location().start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Do not use mixed logical operators in an `unless`.".to_string(),
                    ));
                }
            }
        }
    }
}

fn contains_logical_operator(node: &ruby_prism::Node<'_>) -> bool {
    node.as_and_node().is_some() || node.as_or_node().is_some()
}

fn is_pattern_matching_guard(source: &SourceFile, node: &ruby_prism::Node<'_>) -> bool {
    let loc = node.location();
    let start = loc.start_offset();
    let (line, _col) = source.offset_to_line_col(start);
    if let Some(line_start) = source.line_col_to_offset(line, 0) {
        if let Some(prefix) = source.try_byte_slice(line_start, start) {
            return prefix.trim() == "in";
        }
    }
    false
}

/// Check if the condition has mixed logical operators.
fn contains_mixed_logical_operators(node: &ruby_prism::Node<'_>) -> bool {
    let mut collector = LogicalOperatorCollector::default();
    collector.visit(node);

    // Keep RuboCop's root-node behavior: outer parentheses around the entire
    // condition do not make it a direct `and`/`or` predicate.
    (node.as_or_node().is_some() && collector.has_and())
        || (node.as_and_node().is_some() && collector.has_or())
        || collector.mixed_and()
        || collector.mixed_or()
}

#[derive(Default)]
struct LogicalOperatorCollector {
    and_symbolic: usize,
    and_keyword: usize,
    or_symbolic: usize,
    or_keyword: usize,
}

impl LogicalOperatorCollector {
    fn has_and(&self) -> bool {
        self.and_symbolic + self.and_keyword > 0
    }

    fn has_or(&self) -> bool {
        self.or_symbolic + self.or_keyword > 0
    }

    fn mixed_and(&self) -> bool {
        self.and_symbolic > 0 && self.and_keyword > 0
    }

    fn mixed_or(&self) -> bool {
        self.or_symbolic > 0 && self.or_keyword > 0
    }
}

impl<'pr> Visit<'pr> for LogicalOperatorCollector {
    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        if predicate_operator_predicates::is_logical_and(node) {
            self.and_symbolic += 1;
        } else {
            self.and_keyword += 1;
        }
        ruby_prism::visit_and_node(self, node);
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        if predicate_operator_predicates::is_logical_or(node) {
            self.or_symbolic += 1;
        } else {
            self.or_keyword += 1;
        }
        ruby_prism::visit_or_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forbid_logical_operators_config() -> CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("forbid_logical_operators".to_string()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    crate::cop_fixture_tests!(
        UnlessLogicalOperators,
        "cops/style/unless_logical_operators"
    );

    #[test]
    fn forbid_logical_operators_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &UnlessLogicalOperators,
            include_bytes!(
                "../../../tests/fixtures/cops/style/unless_logical_operators/forbid_logical_operators_offense.rb"
            ),
            forbid_logical_operators_config(),
        );
    }

    #[test]
    fn forbid_logical_operators_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &UnlessLogicalOperators,
            include_bytes!(
                "../../../tests/fixtures/cops/style/unless_logical_operators/forbid_logical_operators_no_offense.rb"
            ),
            forbid_logical_operators_config(),
        );
    }
}
