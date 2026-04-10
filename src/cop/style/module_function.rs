use ruby_prism::Visit;

use crate::cop::shared::access_modifier_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-11, updated 2026-04-07)
///
/// Default style (module_function): FP=0, FN=0, Match=100%.
///
/// Forbidden style variant: FP=490 due to `module_function :foo` (with arguments)
/// being incorrectly flagged. RuboCop's `module_function_node?` matcher is
/// `(send nil? :module_function)` which requires NO arguments.
///
/// Fix: In `forbidden` style, only flag `module_function` when it has no arguments
/// (`call.arguments().is_none()`), matching RuboCop's node matcher.
pub struct ModuleFunction;

impl Cop for ModuleFunction {
    fn name(&self) -> &'static str {
        "Style/ModuleFunction"
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
        let style = config.get_str("EnforcedStyle", "module_function");
        // Autocorrect config key acknowledged (autocorrect not yet implemented)
        let _autocorrect = config.get_bool("Autocorrect", false);
        let mut visitor = ModuleFunctionVisitor {
            cop: self,
            source,
            style,
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct ModuleFunctionVisitor<'a> {
    cop: &'a ModuleFunction,
    source: &'a SourceFile,
    style: &'a str,
    diagnostics: Vec<Diagnostic>,
}

impl<'pr> Visit<'pr> for ModuleFunctionVisitor<'_> {
    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        if let Some(body) = node.body() {
            // Scan the body for `extend self` or `module_function`
            if let Some(stmts) = body.as_statements_node() {
                // RuboCop requires begin_type? (2+ statements in the module body).
                // A module with only one statement (e.g. `module M; extend self; end`)
                // is not flagged because the body is not a begin node in Parser AST.
                let stmt_count = stmts.body().iter().count();
                if stmt_count < 2 {
                    self.visit(&body);
                    return;
                }

                // For module_function style, skip if any private directive exists
                let has_private = self.style == "module_function"
                    && stmts.body().iter().any(|stmt| is_private_directive(&stmt));

                for stmt in stmts.body().iter() {
                    if let Some(call) = stmt.as_call_node() {
                        let method_bytes = call.name().as_slice();

                        if self.style == "module_function"
                            && !has_private
                            && method_bytes == b"extend"
                        {
                            // Check if argument is `self`
                            if call.receiver().is_none() {
                                if let Some(args) = call.arguments() {
                                    let arg_list: Vec<_> = args.arguments().iter().collect();
                                    if arg_list.len() == 1 && arg_list[0].as_self_node().is_some() {
                                        let loc = call.location();
                                        let (line, column) =
                                            self.source.offset_to_line_col(loc.start_offset());
                                        self.diagnostics.push(
                                            self.cop.diagnostic(
                                                self.source,
                                                line,
                                                column,
                                                "Use `module_function` instead of `extend self`."
                                                    .to_string(),
                                            ),
                                        );
                                    }
                                }
                            }
                        } else if self.style == "extend_self" && method_bytes == b"module_function"
                        {
                            // Check if it has no arguments (bare `module_function`)
                            if call.receiver().is_none() && call.arguments().is_none() {
                                let loc = call.location();
                                let (line, column) =
                                    self.source.offset_to_line_col(loc.start_offset());
                                self.diagnostics.push(self.cop.diagnostic(
                                    self.source,
                                    line,
                                    column,
                                    "Use `extend self` instead of `module_function`.".to_string(),
                                ));
                            }
                        } else if self.style == "forbidden" {
                            // RuboCop's module_function_node? matches (send nil? :module_function)
                            // which requires NO arguments. module_function :foo is not flagged.
                            if method_bytes == b"module_function"
                                && call.receiver().is_none()
                                && call.arguments().is_none()
                            {
                                let loc = call.location();
                                let (line, column) =
                                    self.source.offset_to_line_col(loc.start_offset());
                                self.diagnostics.push(
                                    self.cop.diagnostic(
                                        self.source,
                                        line,
                                        column,
                                        "`module_function` and `extend self` are forbidden."
                                            .to_string(),
                                    ),
                                );
                            } else if method_bytes == b"extend" && call.receiver().is_none() {
                                if let Some(args) = call.arguments() {
                                    let arg_list: Vec<_> = args.arguments().iter().collect();
                                    if arg_list.len() == 1 && arg_list[0].as_self_node().is_some() {
                                        let loc = call.location();
                                        let (line, column) =
                                            self.source.offset_to_line_col(loc.start_offset());
                                        self.diagnostics.push(self.cop.diagnostic(
                                            self.source,
                                            line,
                                            column,
                                            "`module_function` and `extend self` are forbidden.".to_string(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            self.visit(&body);
        }
    }
}

/// Returns true if the node is a `private` call with no receiver (bare `private`,
/// `private :method_name`, or `private def ...`).
fn is_private_directive(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        return access_modifier_predicates::is_access_modifier_declaration(&call)
            && call.name().as_slice() == b"private";
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ModuleFunction, "cops/style/module_function");

    fn forbidden_config() -> CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".into(),
            serde_yml::Value::String("forbidden".into()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    #[test]
    fn forbidden_style_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ModuleFunction,
            include_bytes!(
                "../../../tests/fixtures/cops/style/module_function/forbidden_offense.rb"
            ),
            forbidden_config(),
        );
    }

    #[test]
    fn forbidden_style_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ModuleFunction,
            include_bytes!(
                "../../../tests/fixtures/cops/style/module_function/forbidden_no_offense.rb"
            ),
            forbidden_config(),
        );
    }
}
