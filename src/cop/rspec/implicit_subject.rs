use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Flags `is_expected`, `should`, and `should_not` when RuboCop requires
/// explicit subject usage.
///
/// Corpus fix notes:
/// - False negatives came from finding the "enclosing example" by scanning
///   backward through source lines. That let sibling lines such as
///   `its(:count) { ... }` incorrectly exempt the following
///   `its_map { is_expected ... }` call as if it were inside an `its` block.
/// - The same line-based heuristic also guessed single-line examples from raw
///   source text, which is brittle for multiline brace blocks.
/// - Fixed by walking Prism ancestors to the real enclosing example block and
///   deriving `its` / single-line / single-statement behavior from that block.
/// - Added `require_implicit` style support (was completely missing): flags
///   `expect(subject)` in example contexts when style is `require_implicit`.
///   The RuboCop cop uses `explicit_unnamed_subject?` pattern to match
///   `(send nil? :expect (send nil? :subject))`.
pub struct ImplicitSubject;

/// RSpec example method names (it, specify, example, scenario, its, etc.)
const EXAMPLE_METHODS: &[&[u8]] = &[
    b"it",
    b"specify",
    b"example",
    b"scenario",
    b"its",
    b"xit",
    b"xspecify",
    b"xexample",
    b"xscenario",
    b"fit",
    b"fspecify",
    b"fexample",
    b"fscenario",
    b"skip",
    b"pending",
];

fn example_method_name(name: &[u8]) -> Option<&'static [u8]> {
    EXAMPLE_METHODS
        .iter()
        .copied()
        .find(|method| *method == name)
}

fn is_explicit_unnamed_subject(node: &ruby_prism::CallNode<'_>) -> bool {
    // Matches expect(subject) where expect has no receiver and subject has no receiver
    node.name().as_slice() == b"expect"
        && node.receiver().is_none()
        && node.arguments().is_some_and(|args| {
            args.arguments().len() == 1
                && args.arguments().first().is_some_and(|arg| {
                    arg.as_call_node().is_some_and(|inner| {
                        crate::cop::shared::method_dispatch_predicates::is_command(
                            &inner, b"subject",
                        )
                    })
                })
        })
}

fn is_single_line_block(block: &ruby_prism::BlockNode<'_>) -> bool {
    !block.location().as_slice().contains(&b'\n')
}

fn is_single_statement_block(block: &ruby_prism::BlockNode<'_>) -> bool {
    let Some(body) = block.body() else {
        return true;
    };

    if let Some(stmts) = body.as_statements_node() {
        stmts.body().len() <= 1
    } else {
        true
    }
}

impl Cop for ImplicitSubject {
    fn name(&self) -> &'static str {
        "RSpec/ImplicitSubject"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_DEFAULT_INCLUDE
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let mut visitor = ImplicitSubjectVisitor {
            cop: self,
            source,
            enforced_style: config.get_str("EnforcedStyle", "single_line_only"),
            diagnostics,
            ancestors: Vec::new(),
        };
        visitor.visit(&parse_result.node());
    }
}

struct ImplicitSubjectVisitor<'a, 'pr> {
    cop: &'a ImplicitSubject,
    source: &'a SourceFile,
    enforced_style: &'a str,
    diagnostics: &'a mut Vec<Diagnostic>,
    ancestors: Vec<ruby_prism::Node<'pr>>,
}

impl<'a, 'pr> ImplicitSubjectVisitor<'a, 'pr> {
    fn enclosing_example(&self) -> Option<(&'static [u8], ruby_prism::BlockNode<'pr>)> {
        let mut idx = self.ancestors.len().checked_sub(2)?;

        loop {
            if let Some(block) = self.ancestors[idx].as_block_node() {
                if let Some(call_idx) = idx.checked_sub(1) {
                    if let Some(call) = self.ancestors[call_idx].as_call_node() {
                        if call.receiver().is_none() {
                            if let Some(method) = example_method_name(call.name().as_slice()) {
                                return Some((method, block));
                            }
                        }
                    }
                }
            }

            let Some(next_idx) = idx.checked_sub(1) else {
                break;
            };
            idx = next_idx;
        }

        None
    }

    fn add_offense(&mut self, node: &ruby_prism::CallNode<'pr>, msg: &'static str) {
        let loc = node.location();
        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
        self.diagnostics.push(
            self.cop
                .diagnostic(self.source, line, column, msg.to_string()),
        );
    }
}

impl<'a, 'pr> Visit<'pr> for ImplicitSubjectVisitor<'a, 'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {}

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if node.receiver().is_none() {
            let method_name = node.name().as_slice();
            let is_implicit = method_name == b"is_expected"
                || method_name == b"should"
                || method_name == b"should_not";

            if is_implicit {
                let enclosing = self.enclosing_example();

                if let Some((method, _)) = &enclosing {
                    if *method == b"its" {
                        ruby_prism::visit_call_node(self, node);
                        return;
                    }
                }

                if self.enforced_style == "disallow" {
                    self.add_offense(node, "Don't use implicit subject.");
                    ruby_prism::visit_call_node(self, node);
                    return;
                }

                let is_single_line = enclosing
                    .as_ref()
                    .is_some_and(|(_, block)| is_single_line_block(block));

                match self.enforced_style {
                    "single_line_only" => {
                        if !is_single_line {
                            self.add_offense(node, "Don't use implicit subject.");
                        }
                    }
                    "single_statement_only" => {
                        // RuboCop: `!example_of(node)&.body&.begin_type?`
                        // When example_of is nil: !nil&.body&.begin_type? = !nil = true
                        // So "no enclosing example" counts as single-statement (no offense).
                        let is_single_statement = enclosing
                            .as_ref()
                            .is_none_or(|(_, block)| is_single_statement_block(block));
                        if !is_single_line && !is_single_statement {
                            self.add_offense(node, "Don't use implicit subject.");
                        }
                    }
                    _ => {}
                }
            }

            // Handle require_implicit style: flag expect(subject) anywhere.
            // RuboCop checks only explicit_unnamed_subject?(node) with no
            // ancestor/example-block requirement.
            if self.enforced_style == "require_implicit" && is_explicit_unnamed_subject(node) {
                self.add_offense(node, "Don't use explicit subject.");
            }
        }

        ruby_prism::visit_call_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ImplicitSubject, "cops/rspec/implicit_subject");

    #[test]
    fn disallow_style_flags_single_line_too() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("disallow".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"it { is_expected.to eq(1) }\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitSubject, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("Don't use implicit subject"));
    }

    #[test]
    fn disallow_style_allows_its_blocks() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("disallow".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"its(:quality) { is_expected.to be :high }\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitSubject, source, config);
        assert_eq!(
            diags.len(),
            0,
            "its blocks should be exempt even with disallow style"
        );
    }

    #[test]
    fn single_statement_only_allows_single_statement() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("single_statement_only".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"it 'checks' do\n  is_expected.to be_good\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitSubject, source, config);
        assert_eq!(
            diags.len(),
            0,
            "single-statement multi-line should be allowed"
        );
    }

    #[test]
    fn single_statement_only_flags_multi_statement() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("single_statement_only".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"it 'checks' do\n  subject.age = 18\n  is_expected.to be_valid\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitSubject, source, config);
        assert_eq!(diags.len(), 1, "multi-statement should be flagged");
    }

    #[test]
    fn single_line_only_flags_multiline_brace_examples() {
        let source =
            b"it { is_expected.to contain_file(\"#{path}\").with(\n  :ensure => 'file',\n)}\n";
        let diags = crate::testutil::run_cop_full(&ImplicitSubject, source);
        assert_eq!(
            diags.len(),
            1,
            "multiline brace block should not be treated as one-line"
        );
    }

    #[test]
    fn single_line_only_flags_non_example_contexts() {
        // RuboCop: single_line? = example_of(node)&.single_line?
        // When no enclosing example: nil&.single_line? = nil (falsy)
        // → not single-line → offense fires.
        let source = b"before { is_expected.to be_ok }\n";
        let diags = crate::testutil::run_cop_full(&ImplicitSubject, source);
        assert_eq!(
            diags.len(),
            1,
            "non-example contexts should be flagged for single_line_only (default)"
        );
    }

    #[test]
    fn single_statement_only_skips_non_example_contexts() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("single_statement_only".into()),
            )]),
            ..CopConfig::default()
        };
        // RuboCop: single_statement? = !example_of(node)&.body&.begin_type?
        // When no enclosing example: !nil&.body&.begin_type? = !nil = true
        // → counts as single-statement → valid usage → no offense.
        let source = b"describe 'something' do\n  is_expected.to be_valid\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&ImplicitSubject, source, config);
        assert_eq!(
            diags.len(),
            0,
            "non-example contexts should NOT be flagged for single_statement_only"
        );
    }

    #[test]
    fn require_implicit_offense_fixture() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("require_implicit".into()),
            )]),
            ..CopConfig::default()
        };
        crate::testutil::assert_cop_offenses_full_with_config(
            &ImplicitSubject,
            include_bytes!(
                "../../../tests/fixtures/cops/rspec/implicit_subject/require_implicit_offense.rb"
            ),
            config,
        );
    }

    #[test]
    fn require_implicit_no_offense_fixture() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("require_implicit".into()),
            )]),
            ..CopConfig::default()
        };
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ImplicitSubject,
            include_bytes!(
                "../../../tests/fixtures/cops/rspec/implicit_subject/require_implicit_no_offense.rb"
            ),
            config,
        );
    }

    fn single_statement_only_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("single_statement_only".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn single_statement_only_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ImplicitSubject,
            include_bytes!(
                "../../../tests/fixtures/cops/rspec/implicit_subject/single_statement_only_offense.rb"
            ),
            single_statement_only_config(),
        );
    }

    #[test]
    fn single_statement_only_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ImplicitSubject,
            include_bytes!(
                "../../../tests/fixtures/cops/rspec/implicit_subject/single_statement_only_no_offense.rb"
            ),
            single_statement_only_config(),
        );
    }
}
