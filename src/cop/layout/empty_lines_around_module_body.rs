use crate::cop::shared::node_type::MODULE_NODE;
use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Location, Severity};
use crate::parse::source::SourceFile;

/// ## Implementation notes
///
/// This cop originally only handled `no_empty_lines` and `empty_lines` styles.
/// The `empty_lines_except_namespace` and `empty_lines_special` styles were
/// implemented by mirroring RuboCop's `EmptyLinesAroundBody` mixin logic:
/// - `namespace?(body, with_one_child: true)` detects when a module body is a
///   single module/class child (namespace style), requiring `no_empty_lines`
/// - `first_child_requires_empty_line?(body)` detects when the first body child
///   is a def/class/module/visibility modifier, requiring empty lines
pub struct EmptyLinesAroundModuleBody;

impl Cop for EmptyLinesAroundModuleBody {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundModuleBody"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[MODULE_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "no_empty_lines");
        let module_node = match node.as_module_node() {
            Some(m) => m,
            None => return,
        };

        let kw_offset = module_node.module_keyword_loc().start_offset();
        let end_offset = module_node.end_keyword_loc().start_offset();

        match style {
            "empty_lines" => {
                diagnostics.extend(
                    util::check_missing_empty_lines_around_body_with_corrections(
                        self.name(),
                        source,
                        kw_offset,
                        end_offset,
                        "module",
                        corrections,
                    ),
                );
            }
            "empty_lines_except_namespace" => {
                self.check_except_namespace(source, &module_node, diagnostics, corrections);
            }
            "empty_lines_special" => {
                self.check_special(source, &module_node, diagnostics, corrections);
            }
            _ => {
                // "no_empty_lines" (default)
                diagnostics.extend(util::check_empty_lines_around_body_with_corrections(
                    self.name(),
                    source,
                    kw_offset,
                    end_offset,
                    "module",
                    corrections,
                ));
            }
        }
    }
}

impl EmptyLinesAroundModuleBody {
    /// Check if the module body is a namespace (contains only module/class children).
    fn is_namespace_style_body(body: &Option<ruby_prism::Node<'_>>, with_one_child: bool) -> bool {
        let Some(body_node) = body else {
            return false;
        };

        if let Some(stmts) = body_node.as_statements_node() {
            let children: Vec<_> = stmts.body().iter().collect();
            if children.len() == 1 {
                if with_one_child {
                    return children[0].as_module_node().is_some()
                        || children[0].as_class_node().is_some();
                }
                return children[0].as_module_node().is_some()
                    || children[0].as_class_node().is_some();
            }
            if with_one_child {
                return false;
            }
            children
                .iter()
                .all(|c| c.as_module_node().is_some() || c.as_class_node().is_some())
        } else {
            if with_one_child {
                return body_node.as_module_node().is_some() || body_node.as_class_node().is_some();
            }
            body_node.as_module_node().is_some() || body_node.as_class_node().is_some()
        }
    }

    /// Check if the first child of the body requires an empty line.
    /// Matches: def, class, module, or visibility modifier (private/protected/public)
    fn first_child_requires_empty_line(body: &Option<ruby_prism::Node<'_>>) -> bool {
        let Some(body_node) = body else {
            return false;
        };

        // Check the first child based on whether body is statements or single node
        if let Some(stmts) = body_node.as_statements_node() {
            let stmts_vec: Vec<_> = stmts.body().iter().collect();
            if stmts_vec.is_empty() {
                return false;
            }
            // Need to work with the first element - since we have Vec<Node>, we need to handle it
            // For now, check the first element for def/class/module
            let first = &stmts_vec[0];
            Self::node_requires_empty_line(first)
        } else {
            Self::node_requires_empty_line(body_node)
        }
    }

    /// Check if a node requires an empty line before it
    fn node_requires_empty_line(node: &ruby_prism::Node<'_>) -> bool {
        if node.as_def_node().is_some()
            || node.as_class_node().is_some()
            || node.as_module_node().is_some()
        {
            return true;
        }
        if let Some(call) = node.as_call_node() {
            if call.receiver().is_none() {
                let name = call.name();
                let name_bytes = name.as_slice();
                if name_bytes == b"private" || name_bytes == b"protected" || name_bytes == b"public"
                {
                    return true;
                }
            }
        }
        false
    }

    fn check_except_namespace(
        &self,
        source: &SourceFile,
        module_node: &ruby_prism::ModuleNode<'_>,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let kw_offset = module_node.module_keyword_loc().start_offset();
        let end_offset = module_node.end_keyword_loc().start_offset();
        let body = module_node.body();

        let is_namespace = Self::is_namespace_style_body(&body, true);

        if is_namespace {
            diagnostics.extend(util::check_empty_lines_around_body_with_corrections(
                self.name(),
                source,
                kw_offset,
                end_offset,
                "module",
                corrections,
            ));
        } else {
            diagnostics.extend(
                util::check_missing_empty_lines_around_body_with_corrections(
                    self.name(),
                    source,
                    kw_offset,
                    end_offset,
                    "module",
                    corrections,
                ),
            );
        }
    }

    fn check_special(
        &self,
        source: &SourceFile,
        module_node: &ruby_prism::ModuleNode<'_>,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let kw_offset = module_node.module_keyword_loc().start_offset();
        let end_offset = module_node.end_keyword_loc().start_offset();
        let body = module_node.body();

        if body.is_none() {
            return;
        }

        if Self::is_namespace_style_body(&body, true) {
            diagnostics.extend(util::check_empty_lines_around_body_with_corrections(
                self.name(),
                source,
                kw_offset,
                end_offset,
                "module",
                corrections,
            ));
        } else if Self::first_child_requires_empty_line(&body) {
            diagnostics.extend(
                util::check_missing_empty_lines_around_body_with_corrections(
                    self.name(),
                    source,
                    kw_offset,
                    end_offset,
                    "module",
                    corrections,
                ),
            );
        } else {
            let (kw_line, _) = source.offset_to_line_col(kw_offset);
            let (end_line, _) = source.offset_to_line_col(end_offset);

            if end_line > kw_line + 1 {
                let before_end_line = end_line - 1;
                if let Some(line) = util::line_at(source, before_end_line) {
                    if !util::is_blank_line(line) {
                        diagnostics.push(Diagnostic {
                            path: source.path_str().to_string(),
                            location: Location {
                                line: before_end_line,
                                column: 0,
                            },
                            severity: Severity::Convention,
                            cop_name: self.name().to_string(),
                            message: "Empty line missing at module body end.".to_string(),
                            corrected: false,
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        EmptyLinesAroundModuleBody,
        "cops/layout/empty_lines_around_module_body"
    );
    crate::cop_autocorrect_fixture_tests!(
        EmptyLinesAroundModuleBody,
        "cops/layout/empty_lines_around_module_body"
    );

    #[test]
    fn single_line_module_no_offense() {
        let src = b"module Foo; end\n";
        let diags = run_cop_full(&EmptyLinesAroundModuleBody, src);
        assert!(diags.is_empty(), "Single-line module should not trigger");
    }

    #[test]
    fn blank_line_at_both_ends() {
        let src = b"module Foo\n\n  def bar; end\n\nend\n";
        let diags = run_cop_full(&EmptyLinesAroundModuleBody, src);
        assert_eq!(
            diags.len(),
            2,
            "Should flag both beginning and end blank lines"
        );
    }

    #[test]
    fn empty_lines_style_requires_blank_lines() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("empty_lines".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"module Foo\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundModuleBody, src, config);
        assert_eq!(
            diags.len(),
            2,
            "empty_lines style should require blank lines at both ends"
        );
    }

    #[test]
    fn no_empty_lines_style_flags_crlf_empty_lines() {
        let src = b"module Foo\r\n\r\n  X = 1\r\n\r\nend\r\n";
        let diags = run_cop_full(&EmptyLinesAroundModuleBody, src);
        assert_eq!(
            diags.len(),
            2,
            "CRLF empty lines should still be treated as empty"
        );
    }

    #[test]
    fn empty_lines_except_namespace_single_child_namespace() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("empty_lines_except_namespace".into()),
            )]),
            ..CopConfig::default()
        };
        // RuboCop spec: namespace means no empty lines between Parent and Child,
        // but Child's internal content still needs proper formatting
        let src = b"module Parent\n  module Child\n\n    do_something\n\n  end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundModuleBody, src, config);
        assert!(
            diags.is_empty(),
            "Single child namespace should not require empty lines with empty_lines_except_namespace"
        );
    }

    #[test]
    fn empty_lines_except_namespace_multiple_children() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("empty_lines_except_namespace".into()),
            )]),
            ..CopConfig::default()
        };
        // Multiple child modules = not a namespace style
        // Parent needs empty lines around its body (empty_lines style)
        let src = b"module Parent\n\n  module Mom\n\n    do_something\n\n  end\n  module Dad\n\n  end\n\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundModuleBody, src, config);
        assert_eq!(
            diags.len(),
            0,
            "Multiple children should NOT require empty lines when properly formatted"
        );
    }

    #[test]
    fn empty_lines_special_single_child_namespace() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("empty_lines_special".into()),
            )]),
            ..CopConfig::default()
        };
        // Single child namespace - no empty lines at Parent body level
        let src = b"module Parent\n  module Child\n\n    do_something\n\n  end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundModuleBody, src, config);
        assert!(
            diags.is_empty(),
            "Single child namespace should not require empty lines with empty_lines_special"
        );
    }

    #[test]
    fn empty_lines_special_first_child_is_def() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("empty_lines_special".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"module Foo\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundModuleBody, src, config);
        assert_eq!(
            diags.len(),
            2,
            "empty_lines_special with def as first child should require empty lines at beginning and end"
        );
    }

    #[test]
    fn empty_lines_special_first_child_not_def() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("empty_lines_special".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"module Foo\n  include Something\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundModuleBody, src, config);
        assert!(
            diags.len() >= 1,
            "empty_lines_special with non-def first child should require empty line before def"
        );
    }
}
