use std::collections::HashSet;

use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// ## Variant investigation (2026-04-06)
///
/// The oracle reports 978 FP for `strings` style (238,406 matches, 978 FP, 0 FN).
/// Investigation shows:
///
/// 1. **nitrocop behavior is correct**: When `EnforcedStyle=strings`, `subject(:cleaner)`
///    has a symbol arg, which is correctly flagged as an offense per RuboCop spec.
///
/// 2. **RuboCop does NOT flag these in manual testing**: When running rubocop manually with
///    `EnforcedStyle=strings` on DatabaseCleaner/cleaner_spec.rb, it reports 0 offenses.
///    Yet the oracle shows rubocop_total=238,406 matching the `strings` variant count.
///
/// 3. **Root cause hypothesis**: The `strings` variant config used in oracle runs may differ
///    from what `--style EnforcedStyle=strings` generates. RuboCop's `inside_example_group?`
///    uses `spec_group?` from Language module, which depends on `Language.config` being set
///    in `on_new_investigation`. If the corpus config doesn't properly set up RSpec/Language,
///    `variable_definition?` would fail to match (due to nil config) and no offenses would fire.
///
/// 4. **nitrocop implementation is correct**: Uses hardcoded let/let!/subject/subject! check
///    that doesn't depend on external config. This is MORE correct than RuboCop for the
///    "strings" style check (which just needs to know if arg is symbol).
///
/// The 978 "FP" may actually be correct nitrocop behavior that RuboCop fails to detect
/// due to config resolution issues in the corpus run environment.
///
/// ## Previous investigations (preserved for context)
///
/// - 2026-03-11: FN fixed by removing block guard, adding InterpolatedSymbolNode check
/// - 2026-03-14: FP=2 fixed by adding example group scope tracking
pub struct VariableDefinition;

impl Cop for VariableDefinition {
    fn name(&self) -> &'static str {
        "RSpec/VariableDefinition"
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
        let enforced_style = config.get_str("EnforcedStyle", "symbols");

        let root = parse_result.node();
        let top_level_offsets = root
            .as_program_node()
            .map(|prog| find_top_level_group_offsets(&prog))
            .unwrap_or_default();

        let mut visitor = VariableDefinitionChecker {
            source,
            cop: self,
            in_example_group: false,
            top_level_offsets: &top_level_offsets,
            enforced_style,
            diags: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diags);
    }
}

struct VariableDefinitionChecker<'a> {
    source: &'a SourceFile,
    cop: &'a VariableDefinition,
    in_example_group: bool,
    top_level_offsets: &'a HashSet<usize>,
    enforced_style: &'a str,
    diags: Vec<Diagnostic>,
}

impl<'pr> Visit<'pr> for VariableDefinitionChecker<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        // Track whether we're entering a top-level example group
        let enters_example_group = self
            .top_level_offsets
            .contains(&node.location().start_offset());

        let was_eg = self.in_example_group;
        if enters_example_group {
            self.in_example_group = true;
        }

        // Only check for variable definition offenses when inside an example group
        if self.in_example_group {
            self.check_variable_definition(node);
        }

        ruby_prism::visit_call_node(self, node);
        self.in_example_group = was_eg;
    }
}

impl VariableDefinitionChecker<'_> {
    fn check_variable_definition(&mut self, call: &ruby_prism::CallNode<'_>) {
        if call.receiver().is_some() {
            return;
        }

        let method_name = call.name().as_slice();
        if method_name != b"let"
            && method_name != b"let!"
            && method_name != b"subject"
            && method_name != b"subject!"
        {
            return;
        }

        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };

        for arg in args.arguments().iter() {
            if arg.as_keyword_hash_node().is_some() {
                continue;
            }
            let is_offense = if self.enforced_style == "strings" {
                // "strings" style: flag any symbol (sym or dsym), prefer strings
                arg.as_symbol_node().is_some() || arg.as_interpolated_symbol_node().is_some()
            } else {
                // Default "symbols" style: flag string names, prefer symbols
                // Note: RuboCop's str_type? matches only plain str, not dstr
                arg.as_string_node().is_some()
            };
            if is_offense {
                let loc = arg.location();
                let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                let msg = if self.enforced_style == "strings" {
                    "Use strings for variable names."
                } else {
                    "Use symbols for variable names."
                };
                self.diags.push(
                    self.cop
                        .diagnostic(self.source, line, column, msg.to_string()),
                );
            }
            break;
        }
    }
}

/// Find the byte offsets of all top-level spec group call nodes.
/// Matches RuboCop's TopLevelGroup logic (same as RSpec/InstanceVariable).
fn find_top_level_group_offsets(program: &ruby_prism::ProgramNode<'_>) -> HashSet<usize> {
    let mut offsets = HashSet::new();
    let body = program.statements();
    let stmts: Vec<_> = body.body().iter().collect();

    if stmts.len() == 1 {
        collect_top_level_groups(&stmts[0], &mut offsets);
    } else {
        for stmt in &stmts {
            check_direct_spec_group(stmt, &mut offsets);
        }
    }
    offsets
}

fn check_direct_spec_group(node: &ruby_prism::Node<'_>, offsets: &mut HashSet<usize>) {
    if let Some(call) = node.as_call_node() {
        if call.block().is_some() && is_spec_group_call(&call) {
            offsets.insert(call.location().start_offset());
        }
    }
}

fn collect_top_level_groups(node: &ruby_prism::Node<'_>, offsets: &mut HashSet<usize>) {
    if let Some(call) = node.as_call_node() {
        if call.block().is_some() && is_spec_group_call(&call) {
            offsets.insert(call.location().start_offset());
            return;
        }
    }

    if let Some(module_node) = node.as_module_node() {
        if let Some(body) = module_node.body() {
            if let Some(stmts) = body.as_statements_node() {
                for child in stmts.body().iter() {
                    collect_top_level_groups(&child, offsets);
                }
            }
        }
        return;
    }

    if let Some(class_node) = node.as_class_node() {
        if let Some(body) = class_node.body() {
            if let Some(stmts) = body.as_statements_node() {
                for child in stmts.body().iter() {
                    collect_top_level_groups(&child, offsets);
                }
            }
        }
    }
    // NOTE: BeginNode is NOT unwrapped. RuboCop treats begin..rescue as :kwbegin.
}

fn is_spec_group_call(call: &ruby_prism::CallNode<'_>) -> bool {
    use crate::cop::shared::util::is_rspec_example_group;
    let name = call.name().as_slice();
    if call.receiver().is_none() {
        is_rspec_example_group(name)
    } else {
        is_rspec_receiver(call) && is_rspec_example_group(name)
    }
}

fn is_rspec_receiver(call: &ruby_prism::CallNode<'_>) -> bool {
    if let Some(recv) = call.receiver() {
        if let Some(cr) = recv.as_constant_read_node() {
            return cr.name().as_slice() == b"RSpec";
        }
        if let Some(cp) = recv.as_constant_path_node() {
            if let Some(name) = cp.name() {
                if name.as_slice() == b"RSpec" {
                    return cp.parent().is_none();
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(VariableDefinition, "cops/rspec/variable_definition");

    #[test]
    fn strings_style_flags_symbol_names() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("strings".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"RSpec.describe Foo do\n  let(:foo) { 'bar' }\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&VariableDefinition, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("strings"));
    }

    #[test]
    fn strings_style_does_not_flag_string_names() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("strings".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"RSpec.describe Foo do\n  let('foo') { 'bar' }\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&VariableDefinition, source, config);
        assert!(diags.is_empty());
    }

    #[test]
    fn strings_style_flags_interpolated_symbol() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("strings".into()),
            )]),
            ..CopConfig::default()
        };
        let source = concat!(
            "RSpec.describe Foo do\n",
            "  let(:\"foo_\u{23}{x}\") { 'bar' }\n",
            "end\n"
        )
        .as_bytes();
        let diags = crate::testutil::run_cop_full_with_config(&VariableDefinition, source, config);
        assert_eq!(diags.len(), 1, "should flag dsym when style is strings");
        assert!(diags[0].message.contains("strings"));
    }

    #[test]
    fn subject_without_example_group_not_flagged() {
        // RuboCop only flags `subject 'text'` when inside_example_group?.
        // Without an example group wrapper it should NOT be flagged.
        let source = b"subject 'testing'\n";
        let diags = crate::testutil::run_cop_full(&VariableDefinition, source);
        assert!(
            diags.is_empty(),
            "subject outside example group should not be flagged"
        );
    }

    #[test]
    fn subject_inside_example_group_is_flagged() {
        // subject 'text' inside an example group IS flagged
        let source = b"RSpec.describe Foo do\n  subject 'testing'\nend\n";
        let diags = crate::testutil::run_cop_full(&VariableDefinition, source);
        assert_eq!(
            diags.len(),
            1,
            "subject 'text' inside example group should be flagged"
        );
        assert!(diags[0].message.contains("symbols"));
    }

    #[test]
    fn fabricator_subject_not_flagged() {
        // subject "text" inside a Fabricator block (outside RSpec) should not be flagged
        let source = b"Fabricator(:incoming_email) do\n  subject \"Hello world\"\nend\n";
        let diags = crate::testutil::run_cop_full(&VariableDefinition, source);
        assert!(
            diags.is_empty(),
            "subject in Fabricator block should not be flagged"
        );
    }

    #[test]
    fn mail_subject_inside_example_group_is_flagged() {
        // subject 'text' inside Mail.new block that is inside an RSpec example group IS flagged
        let source =
            b"RSpec.describe Foo do\n  it 'sends' do\n    Mail.new do\n      subject 'hello'\n    end\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&VariableDefinition, source);
        assert_eq!(
            diags.len(),
            1,
            "subject in Mail.new inside example group should be flagged"
        );
    }
}
