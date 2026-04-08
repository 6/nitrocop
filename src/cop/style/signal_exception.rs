use ruby_prism::Visit;

use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Corpus investigation (2026-03-27):
///
/// FN=1 remained in `oriuminc__vagrant-ariadne__bb22d52` at
/// `cookbooks-override/ariadne/libraries/helpers.rb:42`.
/// `Style/SignalException` already detects that `fail ... unless ...` call in
/// isolation, in fixtures, and when the repo is linted directly against
/// `bench/corpus/baseline_rubocop.yml`.
///
/// The miss only reproduces through the corpus helper's generated overlay config
/// under `/tmp/nitrocop_corpus_configs/`. That overlay adds an absolute
/// `AllCops: Exclude: /.../cookbooks/**/*` pattern, and nitrocop's global
/// exclude matcher currently overmatches sibling directories such as
/// `cookbooks-override/**/*`, filtering the file out before this cop runs.
///
/// No cop-local detection change is needed here; the correct fix belongs in the
/// config/file-selection pipeline (`src/config/mod.rs` or corpus overlay path
/// handling) so absolute exclude globs keep directory-segment boundaries.
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

/// Check if a call node's receiver is `Kernel` or `::Kernel`.
fn is_kernel_receiver(node: &ruby_prism::CallNode<'_>) -> bool {
    if let Some(recv) = node.receiver() {
        if let Some(cr) = recv.as_constant_read_node() {
            return cr.name().as_slice() == b"Kernel";
        }
        if let Some(cp) = recv.as_constant_path_node() {
            return cp.name().is_some_and(|n| n.as_slice() == b"Kernel");
        }
    }
    false
}

/// Check if a call is bare (no receiver) or has Kernel/::Kernel as receiver.
fn is_bare_or_kernel_call(node: &ruby_prism::CallNode<'_>) -> bool {
    node.receiver().is_none() || is_kernel_receiver(node)
}

impl Visit<'_> for SignalExceptionVisitor<'_> {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'_>) {
        if node.name().as_slice() == b"fail" {
            self.custom_fail_defined = true;
        }
        // Continue visiting children
        ruby_prism::visit_def_node(self, node);
    }

    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'_>) {
        if self.enforced_style == "semantic" {
            // Visit rescue body with in_rescue_body = true
            let prev = self.in_rescue_body;
            self.in_rescue_body = true;
            if let Some(stmts) = node.statements() {
                self.visit_statements_node(&stmts);
            }
            self.in_rescue_body = prev;

            // Visit subsequent rescue clauses
            if let Some(subsequent) = node.subsequent() {
                self.visit_rescue_node(&subsequent);
            }
        } else {
            ruby_prism::visit_rescue_node(self, node);
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'_>) {
        let name = node.name().as_slice();

        match self.enforced_style {
            "only_raise" => {
                // Only bare raise/fail (no receiver)
                if node.receiver().is_none() && name == b"fail" {
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
                if node.receiver().is_none() && name == b"raise" {
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
                if is_bare_or_kernel_call(node) {
                    let loc = node.message_loc().unwrap_or_else(|| node.location());
                    if self.in_rescue_body {
                        // Inside rescue body: fail → offense (use raise)
                        if name == b"fail" {
                            let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                            self.raise_diagnostics.push(self.cop.diagnostic(
                                self.source,
                                line,
                                column,
                                "Use `raise` instead of `fail` to rethrow exceptions.".to_string(),
                            ));
                        }
                        // raise in rescue body → OK, no diagnostic
                    } else {
                        // Outside rescue body: raise → offense (use fail)
                        if name == b"raise" {
                            let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                            self.raise_diagnostics.push(self.cop.diagnostic(
                                self.source,
                                line,
                                column,
                                "Use `fail` instead of `raise` to signal exceptions.".to_string(),
                            ));
                        }
                        // fail outside rescue body → OK, no diagnostic
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
}
