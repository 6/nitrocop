use ruby_prism::Visit;

use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Variant conformance fixes:
///
/// - `EnforcedStyle: only_fail` must treat bare `raise`, `Kernel.raise`, and
///   `::Kernel.raise` as offenses. The previous implementation only matched
///   receiverless calls, which missed the corpus `Kernel.raise` family.
/// - `EnforcedStyle: semantic` must model rescue scope boundaries the same way
///   RuboCop does. Two variant mismatches came from a single gap:
///   `in_rescue_body` leaked through nested `begin/rescue` nodes, so inner
///   begin bodies were treated like rethrow sites, and inline
///   `expr rescue raise/fail` rescue modifiers were never given rescue-body
///   semantics at all.
/// - The receiver check intentionally matches only bare `Kernel` and `::Kernel`
///   (not qualified paths like `Foo::Kernel`) to mirror RuboCop's
///   `(const {nil? cbase} :Kernel)` matcher.
pub struct SignalException;

impl Cop for SignalException {
    fn name(&self) -> &'static str {
        "Style/SignalException"
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
        let enforced_style = config.get_str("EnforcedStyle", "only_raise");

        let mut visitor = SignalExceptionVisitor {
            cop: self,
            source,
            enforced_style,
            custom_fail_defined: false,
            in_rescue_body: false,
            pending_fail_diagnostics: Vec::new(),
            raise_diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());

        // Emit raise diagnostics unconditionally (only_fail style, or semantic)
        diagnostics.extend(visitor.raise_diagnostics);

        // Only emit fail diagnostics if no custom fail method is defined
        if !visitor.custom_fail_defined {
            diagnostics.extend(visitor.pending_fail_diagnostics);
        }
    }
}

struct SignalExceptionVisitor<'a> {
    cop: &'a SignalException,
    source: &'a SourceFile,
    enforced_style: &'a str,
    custom_fail_defined: bool,
    /// Whether we're currently inside a rescue body (for semantic style)
    in_rescue_body: bool,
    /// Diagnostics for bare `fail` calls (only emitted if no custom fail defined)
    pending_fail_diagnostics: Vec<Diagnostic>,
    /// Diagnostics for bare `raise` calls (always emitted for only_fail/semantic style)
    raise_diagnostics: Vec<Diagnostic>,
}

/// Check if a call node's receiver is bare `Kernel` or `::Kernel`.
fn is_kernel_receiver(node: &ruby_prism::CallNode<'_>) -> bool {
    if let Some(recv) = node.receiver() {
        if let Some(cr) = recv.as_constant_read_node() {
            return cr.name().as_slice() == b"Kernel";
        }
        if let Some(cp) = recv.as_constant_path_node() {
            return cp.parent().is_none() && cp.name().is_some_and(|n| n.as_slice() == b"Kernel");
        }
    }
    false
}

/// Check if a call is bare (no receiver) or has bare `Kernel`/`::Kernel` as
/// receiver, and the method name matches.
fn is_command_or_kernel_call(node: &ruby_prism::CallNode<'_>, name: &[u8]) -> bool {
    node.name().as_slice() == name && (node.receiver().is_none() || is_kernel_receiver(node))
}

impl Visit<'_> for SignalExceptionVisitor<'_> {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'_>) {
        if node.name().as_slice() == b"fail" {
            self.custom_fail_defined = true;
        }
        // Continue visiting children
        ruby_prism::visit_def_node(self, node);
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'_>) {
        if self.enforced_style == "semantic" && node.rescue_clause().is_some() {
            let prev = self.in_rescue_body;
            self.in_rescue_body = false;

            if let Some(stmts) = node.statements() {
                self.visit_statements_node(&stmts);
            }
            if let Some(rescue_clause) = node.rescue_clause() {
                self.visit_rescue_node(&rescue_clause);
            }
            if let Some(else_clause) = node.else_clause() {
                self.visit_else_node(&else_clause);
            }
            if let Some(ensure_clause) = node.ensure_clause() {
                self.visit_ensure_node(&ensure_clause);
            }

            self.in_rescue_body = prev;
        } else {
            ruby_prism::visit_begin_node(self, node);
        }
    }

    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'_>) {
        if self.enforced_style == "semantic" {
            let prev = self.in_rescue_body;
            self.in_rescue_body = true;
            if let Some(stmts) = node.statements() {
                self.visit_statements_node(&stmts);
            }
            self.in_rescue_body = prev;

            if let Some(subsequent) = node.subsequent() {
                self.visit_rescue_node(&subsequent);
            }
        } else {
            ruby_prism::visit_rescue_node(self, node);
        }
    }

    fn visit_rescue_modifier_node(&mut self, node: &ruby_prism::RescueModifierNode<'_>) {
        if self.enforced_style == "semantic" {
            let prev = self.in_rescue_body;

            self.in_rescue_body = false;
            self.visit(&node.expression());

            self.in_rescue_body = true;
            self.visit(&node.rescue_expression());

            self.in_rescue_body = prev;
        } else {
            ruby_prism::visit_rescue_modifier_node(self, node);
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'_>) {
        let name = node.name().as_slice();

        match self.enforced_style {
            "only_raise" => {
                if is_command_or_kernel_call(node, b"fail") {
                    let loc = node.message_loc().unwrap_or_else(|| node.location());
                    let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                    self.pending_fail_diagnostics.push(self.cop.diagnostic(
                        self.source,
                        line,
                        column,
                        "Use `raise` instead of `fail` to rethrow exceptions.".to_string(),
                    ));
                }
            }
            "only_fail" => {
                if is_command_or_kernel_call(node, b"raise") {
                    let loc = node.message_loc().unwrap_or_else(|| node.location());
                    let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                    self.raise_diagnostics.push(self.cop.diagnostic(
                        self.source,
                        line,
                        column,
                        "Use `fail` instead of `raise` to rethrow exceptions.".to_string(),
                    ));
                }
            }
            "semantic" => {
                if is_command_or_kernel_call(node, b"fail")
                    || is_command_or_kernel_call(node, b"raise")
                {
                    let loc = node.message_loc().unwrap_or_else(|| node.location());
                    if self.in_rescue_body {
                        if name == b"fail" {
                            let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                            self.raise_diagnostics.push(self.cop.diagnostic(
                                self.source,
                                line,
                                column,
                                "Use `raise` instead of `fail` to rethrow exceptions.".to_string(),
                            ));
                        }
                    } else if name == b"raise" {
                        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                        self.raise_diagnostics.push(self.cop.diagnostic(
                            self.source,
                            line,
                            column,
                            "Use `fail` instead of `raise` to signal exceptions.".to_string(),
                        ));
                    }
                }
            }
            _ => {}
        }

        // Continue visiting children
        ruby_prism::visit_call_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{
        assert_cop_no_offenses_full_with_config, assert_cop_offenses_full_with_config,
        run_cop_full_with_config,
    };

    crate::cop_fixture_tests!(SignalException, "cops/style/signal_exception");

    fn semantic_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("semantic".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn semantic_offense() {
        assert_cop_offenses_full_with_config(
            &SignalException,
            include_bytes!(
                "../../../tests/fixtures/cops/style/signal_exception/semantic_offense.rb"
            ),
            semantic_config(),
        );
    }

    #[test]
    fn semantic_no_offense() {
        assert_cop_no_offenses_full_with_config(
            &SignalException,
            include_bytes!(
                "../../../tests/fixtures/cops/style/signal_exception/semantic_no_offense.rb"
            ),
            semantic_config(),
        );
    }

    #[test]
    fn config_only_fail() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("only_fail".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"raise RuntimeError, \"msg\"\n";
        let diags = run_cop_full_with_config(&SignalException, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("Use `fail`"));
    }

    #[test]
    fn only_fail_offense() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("only_fail".into()),
            )]),
            ..CopConfig::default()
        };

        assert_cop_offenses_full_with_config(
            &SignalException,
            include_bytes!(
                "../../../tests/fixtures/cops/style/signal_exception/only_fail_offense.rb"
            ),
            config,
        );
    }
}
