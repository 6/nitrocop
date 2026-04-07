use crate::cop::shared::util::{RSPEC_DEFAULT_INCLUDE, is_camel_case, is_rspec_example_group};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;
use std::sync::OnceLock;

/// Checks memoized helper names against RuboCop's RSpec variable-name styles.
/// Fixed a false negative where operator-only names like `subject(:==)` were
/// treated as valid `snake_case` because the shared helper allows `=` suffixes.
/// Fixed false positives for blank helper names such as `let(:"")`, which
/// RuboCop accepts and which appear in rswag request-spec parameter helpers.
/// Fixed false positives for lowercase-starting camelCase names (e.g., `userName`)
/// which RuboCop's camelCase regex accepts but the old is_camel_case() rejected.
/// Fixed false positives for names with trailing sigils (e.g., `number?`) which
/// RuboCop's camelCase regex allows via the [!?=]? suffix but the old is_camel_case()
/// rejected by treating the trailing ? as a mid-string non-alphanumeric character.
pub struct VariableName;

impl Cop for VariableName {
    fn name(&self) -> &'static str {
        "RSpec/VariableName"
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
        _code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Config: EnforcedStyle — "snake_case" (default) or "camelCase"
        let enforced_camel_case = config.get_str("EnforcedStyle", "snake_case") == "camelCase";
        // Config: AllowedPatterns — regex patterns to exclude
        let allowed_patterns = config
            .get_string_array("AllowedPatterns")
            .unwrap_or_default()
            .into_iter()
            .filter_map(|pattern| regex::Regex::new(&pattern).ok())
            .collect();

        let mut visitor = VariableNameVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            in_spec_group_root: false,
            enforced_camel_case,
            allowed_patterns,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct VariableNameVisitor<'a> {
    cop: &'a VariableName,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    // Mirrors RuboCop's InsideExampleGroup mixin: this is true only when the
    // current top-level root expression is an RSpec example/shared group.
    in_spec_group_root: bool,
    enforced_camel_case: bool,
    allowed_patterns: Vec<regex::Regex>,
}

impl VariableNameVisitor<'_> {
    fn style_name(&self) -> &'static str {
        if self.enforced_camel_case {
            "camelCase"
        } else {
            "snake_case"
        }
    }

    fn check_style(&self, name: &[u8]) -> bool {
        if self.enforced_camel_case {
            is_camel_case(name)
        } else {
            is_rubocop_snake_case(name)
        }
    }

    fn matches_allowed_pattern(&self, name: &[u8]) -> bool {
        let name_str = std::str::from_utf8(name).unwrap_or("");
        self.allowed_patterns
            .iter()
            .any(|pattern| pattern.is_match(name_str))
    }
}

fn is_rubocop_snake_case(name: &[u8]) -> bool {
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();

    let name = match std::str::from_utf8(name) {
        Ok(name) => name,
        Err(_) => return false,
    };

    REGEX
        .get_or_init(|| regex::Regex::new(r"^@{0,2}[\p{Ll}\d_]+[!?=]?$").unwrap())
        .is_match(name)
}

impl<'pr> Visit<'pr> for VariableNameVisitor<'_> {
    fn visit_program_node(&mut self, node: &ruby_prism::ProgramNode<'pr>) {
        for stmt in node.statements().body().iter() {
            let was = self.in_spec_group_root;
            self.in_spec_group_root = is_spec_group_root_statement(&stmt);
            self.visit(&stmt);
            self.in_spec_group_root = was;
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if self.in_spec_group_root
            && node.receiver().is_none()
            && is_variable_definition_method(node.name().as_slice())
        {
            if let Some(args) = node.arguments() {
                for arg in args.arguments().iter() {
                    if arg.as_keyword_hash_node().is_some() {
                        continue;
                    }

                    let name_owned: Option<Vec<u8>> = if let Some(sym) = arg.as_symbol_node() {
                        Some(sym.unescaped().to_vec())
                    } else {
                        arg.as_string_node().map(|s| s.unescaped().to_vec())
                    };

                    if let Some(name) = name_owned {
                        if name.is_empty() {
                            break;
                        }

                        if !self.matches_allowed_pattern(&name) && !self.check_style(&name) {
                            let loc = arg.location();
                            let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                            self.diagnostics.push(self.cop.diagnostic(
                                self.source,
                                line,
                                column,
                                format!("Use {} for variable names.", self.style_name()),
                            ));
                        }
                    }
                    break;
                }
            }
        }

        ruby_prism::visit_call_node(self, node);
    }
}

fn is_variable_definition_method(name: &[u8]) -> bool {
    matches!(name, b"let" | b"let!" | b"subject" | b"subject!")
}

fn is_spec_group_root_statement(node: &ruby_prism::Node<'_>) -> bool {
    node.as_call_node()
        .is_some_and(|call| is_spec_group_call(&call))
}

fn is_spec_group_call(call: &ruby_prism::CallNode<'_>) -> bool {
    if call.block().is_none() {
        return false;
    }

    let method_name = call.name().as_slice();
    if !is_rspec_example_group(method_name) {
        return false;
    }

    match call.receiver() {
        None => true,
        Some(receiver) => {
            if let Some(cr) = receiver.as_constant_read_node() {
                cr.name().as_slice() == b"RSpec"
            } else if let Some(cp) = receiver.as_constant_path_node() {
                cp.parent().is_none() && cp.name().is_some_and(|n| n.as_slice() == b"RSpec")
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(VariableName, "cops/rspec/variable_name");

    #[test]
    fn camel_case_style_flags_snake_case() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("camelCase".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"RSpec.describe Foo do\n  let(:my_var) { 'x' }\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&VariableName, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("camelCase"));
    }

    #[test]
    fn camel_case_style_accepts_lowercase_starting_camel() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("camelCase".into()),
            )]),
            ..CopConfig::default()
        };
        // RuboCop's camelCase regex accepts lowercase-starting names like userName
        let source = b"RSpec.describe Foo do\n  let(:userName) { 'x' }\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&VariableName, source, config);
        assert!(
            diags.is_empty(),
            "camelCase style should accept lowercase-starting names like userName"
        );
    }

    #[test]
    fn allowed_patterns_skips_matching() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "AllowedPatterns".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("^myVar".into())]),
            )]),
            ..CopConfig::default()
        };
        let source = b"RSpec.describe Foo do\n  let(:myVar) { 'x' }\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&VariableName, source, config);
        assert!(
            diags.is_empty(),
            "AllowedPatterns should skip matching variable names"
        );
    }
}
