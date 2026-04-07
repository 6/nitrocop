use crate::cop::shared::access_modifier_predicates;
use crate::cop::shared::node_type::{CLASS_NODE, SINGLETON_CLASS_NODE};
use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Investigation: 431 FN across 106 repos were caused by missing
/// `SingletonClassNode` handling. The dominant pattern is `class << self`
/// blocks with empty lines at beginning/end. RuboCop handles this via
/// `on_sclass` in addition to `on_class`. Fixed by adding
/// `SINGLETON_CLASS_NODE` to `interested_node_types` and extracting
/// keyword/end offsets from `SingletonClassNode`.
///
/// Investigation: 28 FN (24 from pat__thinking-sphinx) caused by multiline
/// class declarations (`class Foo <\n  Bar`). The body start was calculated
/// from the `class` keyword line, not the superclass end line. Fixed by
/// using `superclass.location().end_offset()` as the keyword offset when
/// a superclass is present, so the utility correctly identifies the first
/// body line after the superclass.
///
/// Investigation: The `empty_lines_except_namespace` and `empty_lines_special`
/// styles were not implemented - they fell through to `no_empty_lines` behavior,
/// causing massive FP/FN divergence. Fixed by implementing proper branching:
/// - `empty_lines_except_namespace`: uses `no_empty_lines` for namespace bodies
///   (single class/module child), `empty_lines` otherwise
/// - `empty_lines_special`: uses `no_empty_lines` for namespace bodies;
///   at beginning, uses `empty_lines` only if first child requires it
///   (def/class/module/public/private/protected), else `no_empty_lines`;
///   always uses `empty_lines` at end
pub struct EmptyLinesAroundClassBody;

impl Cop for EmptyLinesAroundClassBody {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundClassBody"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CLASS_NODE, SINGLETON_CLASS_NODE]
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
        let (kw_offset, end_offset, body) = if let Some(class_node) = node.as_class_node() {
            // For multiline class declarations (class Foo <\n  Bar), use the
            // superclass end line so the utility correctly identifies the body start.
            let kw = if let Some(superclass) = class_node.superclass() {
                superclass.location().end_offset().saturating_sub(1)
            } else {
                class_node.class_keyword_loc().start_offset()
            };
            (
                kw,
                class_node.end_keyword_loc().start_offset(),
                class_node.body(),
            )
        } else if let Some(sclass_node) = node.as_singleton_class_node() {
            (
                sclass_node.class_keyword_loc().start_offset(),
                sclass_node.end_keyword_loc().start_offset(),
                sclass_node.body(),
            )
        } else {
            return;
        };

        match style {
            "empty_lines" => {
                diagnostics.extend(
                    util::check_missing_empty_lines_around_body_with_corrections(
                        self.name(),
                        source,
                        kw_offset,
                        end_offset,
                        "class",
                        corrections,
                    ),
                );
            }
            "beginning_only" => {
                // Require blank line at beginning, flag blank at end
                let mut diags = util::check_missing_empty_lines_around_body(
                    self.name(),
                    source,
                    kw_offset,
                    end_offset,
                    "class",
                );
                // Keep only "beginning" diagnostics
                diags.retain(|d| d.message.contains("beginning"));
                // Also flag extra blank at end
                let extra_end = util::check_empty_lines_around_body(
                    self.name(),
                    source,
                    kw_offset,
                    end_offset,
                    "class",
                );
                diags.extend(extra_end.into_iter().filter(|d| d.message.contains("end")));
                diagnostics.extend(diags);
            }
            "ending_only" => {
                // Require blank line at end, flag blank at beginning
                let mut diags = util::check_missing_empty_lines_around_body(
                    self.name(),
                    source,
                    kw_offset,
                    end_offset,
                    "class",
                );
                // Keep only "end" diagnostics
                diags.retain(|d| d.message.contains("end"));
                // Also flag extra blank at beginning
                let extra_begin = util::check_empty_lines_around_body(
                    self.name(),
                    source,
                    kw_offset,
                    end_offset,
                    "class",
                );
                diags.extend(
                    extra_begin
                        .into_iter()
                        .filter(|d| d.message.contains("beginning")),
                );
                diagnostics.extend(diags);
            }
            "empty_lines_except_namespace" => {
                // Namespace (single class/module child) -> no_empty_lines
                // Otherwise -> empty_lines
                if is_namespace_body(&body) {
                    diagnostics.extend(util::check_empty_lines_around_body_with_corrections(
                        self.name(),
                        source,
                        kw_offset,
                        end_offset,
                        "class",
                        corrections,
                    ));
                } else {
                    diagnostics.extend(
                        util::check_missing_empty_lines_around_body_with_corrections(
                            self.name(),
                            source,
                            kw_offset,
                            end_offset,
                            "class",
                            corrections,
                        ),
                    );
                }
            }
            "empty_lines_special" => {
                // Namespace (single class/module child) -> no_empty_lines
                // Otherwise:
                //   - At beginning: empty_lines if first child requires it, else no_empty_lines
                //   - At end: always empty_lines
                if is_namespace_body(&body) {
                    diagnostics.extend(util::check_empty_lines_around_body_with_corrections(
                        self.name(),
                        source,
                        kw_offset,
                        end_offset,
                        "class",
                        corrections,
                    ));
                } else {
                    // At beginning: check if first child requires empty line
                    if first_child_requires_empty_line(&body) {
                        // empty_lines at beginning
                        let mut diags = util::check_missing_empty_lines_around_body(
                            self.name(),
                            source,
                            kw_offset,
                            end_offset,
                            "class",
                        );
                        diags.retain(|d| d.message.contains("beginning"));
                        diagnostics.extend(diags);
                    }
                    // At end: always empty_lines
                    let mut end_diags = util::check_missing_empty_lines_around_body(
                        self.name(),
                        source,
                        kw_offset,
                        end_offset,
                        "class",
                    );
                    end_diags.retain(|d| d.message.contains("end"));
                    diagnostics.extend(end_diags);
                }
            }
            _ => {
                // "no_empty_lines" (default)
                diagnostics.extend(util::check_empty_lines_around_body_with_corrections(
                    self.name(),
                    source,
                    kw_offset,
                    end_offset,
                    "class",
                    corrections,
                ));
            }
        }
    }
}

/// Check if the body is a "namespace" for `empty_lines_except_namespace` purposes.
/// A namespace is a body that is:
/// - A single class or module node, OR
/// - A begin/statements node with exactly one child that is a class or module
fn is_namespace_body(body: &Option<ruby_prism::Node<'_>>) -> bool {
    match body {
        None => false,
        Some(node) => {
            // Direct class or module child
            if node.as_class_node().is_some() || node.as_module_node().is_some() {
                return true;
            }
            // StatementsNode or BeginNode with single class/module child
            let children: Vec<_> = if let Some(stmts) = node.as_statements_node() {
                stmts.body().iter().collect()
            } else if let Some(begin) = node.as_begin_node() {
                if let Some(stmts) = begin.statements() {
                    stmts.body().iter().collect()
                } else {
                    vec![]
                }
            } else {
                return false;
            };
            children.len() == 1
                && (children[0].as_class_node().is_some() || children[0].as_module_node().is_some())
        }
    }
}

/// Check if the first child of the body requires an empty line before it.
/// Per RuboCop: `{any_def class module (send nil? {:private :protected :public})}`
fn first_child_requires_empty_line(body: &Option<ruby_prism::Node<'_>>) -> bool {
    // Helper to check if a node is a bare access modifier (private/protected/public)
    fn is_bare_access_modifier(node: &ruby_prism::Node<'_>) -> bool {
        if let Some(call_node) = node.as_call_node() {
            call_node.receiver().is_none()
                && call_node.arguments().is_none()
                && call_node.block().is_none()
                && access_modifier_predicates::is_access_modifier_name(call_node.name().as_slice())
        } else {
            false
        }
    }

    match body {
        None => false,
        Some(node) => {
            if let Some(stmts) = node.as_statements_node() {
                let children: Vec<_> = stmts.body().iter().collect();
                if children.is_empty() {
                    return false;
                }
                let first = &children[0];
                first.as_class_node().is_some()
                    || first.as_module_node().is_some()
                    || first.as_def_node().is_some()
                    || is_bare_access_modifier(first)
            } else if let Some(begin) = node.as_begin_node() {
                if let Some(stmts) = begin.statements() {
                    let children: Vec<_> = stmts.body().iter().collect();
                    if children.is_empty() {
                        return false;
                    }
                    let first = &children[0];
                    first.as_class_node().is_some()
                        || first.as_module_node().is_some()
                        || first.as_def_node().is_some()
                        || is_bare_access_modifier(first)
                } else {
                    false
                }
            } else {
                // Single node body
                node.as_class_node().is_some()
                    || node.as_module_node().is_some()
                    || node.as_def_node().is_some()
                    || is_bare_access_modifier(node)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        EmptyLinesAroundClassBody,
        "cops/layout/empty_lines_around_class_body"
    );
    crate::cop_autocorrect_fixture_tests!(
        EmptyLinesAroundClassBody,
        "cops/layout/empty_lines_around_class_body"
    );

    #[test]
    fn single_line_class_no_offense() {
        let src = b"class Foo; end\n";
        let diags = run_cop_full(&EmptyLinesAroundClassBody, src);
        assert!(diags.is_empty(), "Single-line class should not trigger");
    }

    #[test]
    fn blank_line_at_both_ends() {
        let src = b"class Foo\n\n  def bar; end\n\nend\n";
        let diags = run_cop_full(&EmptyLinesAroundClassBody, src);
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
        let src = b"class Foo\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundClassBody, src, config);
        assert_eq!(
            diags.len(),
            2,
            "empty_lines style should require blank lines at both ends"
        );
    }

    #[test]
    fn beginning_only_style() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("beginning_only".into()),
            )]),
            ..CopConfig::default()
        };
        // No blank at beginning => flag missing beginning blank
        let src = b"class Foo\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundClassBody, src, config);
        assert!(
            diags.iter().any(|d| d.message.contains("beginning")),
            "beginning_only should flag missing blank at beginning"
        );
    }
}
