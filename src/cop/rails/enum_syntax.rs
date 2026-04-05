use crate::cop::shared::node_type::{ASSOC_NODE, CALL_NODE, KEYWORD_HASH_NODE, SYMBOL_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

pub struct EnumSyntax;

impl Cop for EnumSyntax {
    fn name(&self) -> &'static str {
        "Rails/EnumSyntax"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[ASSOC_NODE, CALL_NODE, KEYWORD_HASH_NODE, SYMBOL_NODE]
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
        // minimum_target_rails_version 7.0
        if !config.rails_version_at_least(7.0) {
            return;
        }

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        if call.receiver().is_some() {
            return;
        }

        if call.name().as_slice() != b"enum" {
            return;
        }

        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };

        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.is_empty() {
            return;
        }

        // Old syntax: enum status: { active: 0 }
        // The first argument is a KeywordHashNode containing status: { ... }
        // New syntax: enum :status, { active: 0 }
        // The first argument is a SymbolNode
        if arg_list[0].as_symbol_node().is_some() {
            // Already using new syntax
            return;
        }

        // Check if first arg is a keyword hash with a symbol key mapped to a hash value
        if let Some(kw) = arg_list[0].as_keyword_hash_node() {
            for elem in kw.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    if assoc.key().as_symbol_node().is_some() {
                        // This is old syntax: enum status: { ... } or enum status: [...]
                        let loc = node.location();
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Use Rails 7+ enum syntax: `enum :status, { active: 0 }`.".to_string(),
                        ));
                    }
                }
            }
        }
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
            &EnumSyntax,
            include_bytes!("../../../tests/fixtures/cops/rails/enum_syntax/offense.rb"),
            config_with_rails(7.0),
        );
    }

    #[test]
    fn no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &EnumSyntax,
            include_bytes!("../../../tests/fixtures/cops/rails/enum_syntax/no_offense.rb"),
            config_with_rails(7.0),
        );
    }

    #[test]
    fn skipped_when_rails_below_7() {
        let source = b"enum status: { active: 0, archived: 1 }\n";
        let diagnostics = crate::testutil::run_cop_full_internal(
            &EnumSyntax,
            source,
            config_with_rails(6.1),
            "test.rb",
        );
        assert!(diagnostics.is_empty(), "Should not fire on Rails < 7.0");
    }

    #[test]
    fn skipped_when_no_rails_version() {
        let source = b"enum status: { active: 0, archived: 1 }\n";
        let diagnostics = crate::testutil::run_cop_full_internal(
            &EnumSyntax,
            source,
            CopConfig::default(),
            "test.rb",
        );
        assert!(
            diagnostics.is_empty(),
            "Should not fire when TargetRailsVersion defaults to 5.0"
        );
    }
}
