use crate::cop::shared::access_modifier_predicates;
use crate::cop::shared::node_type::{CLASS_NODE, SINGLETON_CLASS_NODE};
use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Location, Severity};
use crate::parse::source::SourceFile;

/// Mirrors RuboCop's `EmptyLinesAroundBody` mixin for classes and `class << self`.
///
/// Key parity fixes:
/// - handles singleton classes and multiline superclass headers
/// - treats non-default styles as no-ops for empty/comment-only bodies (`body: nil`)
/// - preserves RuboCop's 2-line-body behavior, where a body starting on the
///   header line yields a single `beginning` offense
/// - matches `empty_lines_special`'s deferred scan exactly: skip comment lines
///   when searching upward, but only a literally empty line satisfies the
///   separator, so whitespace-only lines still offend
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
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
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
        let (first_line, _) = source.offset_to_line_col(kw_offset);
        let (last_line, _) = source.offset_to_line_col(end_offset);

        if first_line == last_line {
            return;
        }

        if body.is_none() && style != "no_empty_lines" {
            return;
        }

        match style {
            "empty_lines" => {
                check_boundary_styles(
                    self.name(),
                    source,
                    first_line,
                    last_line,
                    "class",
                    BodyBoundaryStyle::EmptyLines,
                    BodyBoundaryStyle::EmptyLines,
                    diagnostics,
                    corrections.as_deref_mut(),
                );
            }
            "beginning_only" => {
                check_boundary_styles(
                    self.name(),
                    source,
                    first_line,
                    last_line,
                    "class",
                    BodyBoundaryStyle::EmptyLines,
                    BodyBoundaryStyle::NoEmptyLines,
                    diagnostics,
                    corrections.as_deref_mut(),
                );
            }
            "ending_only" => {
                check_boundary_styles(
                    self.name(),
                    source,
                    first_line,
                    last_line,
                    "class",
                    BodyBoundaryStyle::NoEmptyLines,
                    BodyBoundaryStyle::EmptyLines,
                    diagnostics,
                    corrections.as_deref_mut(),
                );
            }
            "empty_lines_except_namespace" => {
                if is_namespace_body(&body) {
                    check_boundary_styles(
                        self.name(),
                        source,
                        first_line,
                        last_line,
                        "class",
                        BodyBoundaryStyle::NoEmptyLines,
                        BodyBoundaryStyle::NoEmptyLines,
                        diagnostics,
                        corrections.as_deref_mut(),
                    );
                } else {
                    check_boundary_styles(
                        self.name(),
                        source,
                        first_line,
                        last_line,
                        "class",
                        BodyBoundaryStyle::EmptyLines,
                        BodyBoundaryStyle::EmptyLines,
                        diagnostics,
                        corrections.as_deref_mut(),
                    );
                }
            }
            "empty_lines_special" => {
                if is_namespace_body(&body) {
                    check_boundary_styles(
                        self.name(),
                        source,
                        first_line,
                        last_line,
                        "class",
                        BodyBoundaryStyle::NoEmptyLines,
                        BodyBoundaryStyle::NoEmptyLines,
                        diagnostics,
                        corrections.as_deref_mut(),
                    );
                } else {
                    let mut emitted_locations = std::collections::HashSet::new();
                    if first_child_requires_empty_line(&body) {
                        check_beginning_boundary_style(
                            self.name(),
                            source,
                            first_line,
                            "class",
                            BodyBoundaryStyle::EmptyLines,
                            diagnostics,
                            corrections.as_deref_mut(),
                            &mut emitted_locations,
                        );
                    } else {
                        check_beginning_boundary_style(
                            self.name(),
                            source,
                            first_line,
                            "class",
                            BodyBoundaryStyle::NoEmptyLines,
                            diagnostics,
                            corrections.as_deref_mut(),
                            &mut emitted_locations,
                        );
                        check_deferred_empty_line(
                            self.name(),
                            source,
                            body.as_ref(),
                            diagnostics,
                            corrections.as_deref_mut(),
                            &mut emitted_locations,
                        );
                    }
                    check_ending_boundary_style(
                        self.name(),
                        source,
                        last_line,
                        "class",
                        BodyBoundaryStyle::EmptyLines,
                        diagnostics,
                        corrections.as_deref_mut(),
                        &mut emitted_locations,
                    );
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

#[derive(Clone, Copy)]
enum BodyBoundaryStyle {
    NoEmptyLines,
    EmptyLines,
}

#[allow(clippy::too_many_arguments)]
fn check_boundary_styles(
    cop_name: &'static str,
    source: &SourceFile,
    first_line: usize,
    last_line: usize,
    body_kind: &str,
    beginning_style: BodyBoundaryStyle,
    ending_style: BodyBoundaryStyle,
    diagnostics: &mut Vec<Diagnostic>,
    mut corrections: Option<&mut Vec<crate::correction::Correction>>,
) {
    let mut emitted_locations = std::collections::HashSet::new();
    check_beginning_boundary_style(
        cop_name,
        source,
        first_line,
        body_kind,
        beginning_style,
        diagnostics,
        corrections.as_deref_mut(),
        &mut emitted_locations,
    );
    check_ending_boundary_style(
        cop_name,
        source,
        last_line,
        body_kind,
        ending_style,
        diagnostics,
        corrections,
        &mut emitted_locations,
    );
}

#[allow(clippy::too_many_arguments)]
fn check_beginning_boundary_style(
    cop_name: &'static str,
    source: &SourceFile,
    first_line: usize,
    body_kind: &str,
    style: BodyBoundaryStyle,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: Option<&mut Vec<crate::correction::Correction>>,
    emitted_locations: &mut std::collections::HashSet<(usize, usize)>,
) {
    let report_line = first_line + 1;
    let message = match style {
        BodyBoundaryStyle::NoEmptyLines => {
            format!("Extra empty line detected at {body_kind} body beginning.")
        }
        BodyBoundaryStyle::EmptyLines => {
            format!("Empty line missing at {body_kind} body beginning.")
        }
    };

    check_boundary_line(
        cop_name,
        source,
        report_line,
        report_line,
        style,
        message,
        diagnostics,
        corrections,
        emitted_locations,
    );
}

#[allow(clippy::too_many_arguments)]
fn check_ending_boundary_style(
    cop_name: &'static str,
    source: &SourceFile,
    last_line: usize,
    body_kind: &str,
    style: BodyBoundaryStyle,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: Option<&mut Vec<crate::correction::Correction>>,
    emitted_locations: &mut std::collections::HashSet<(usize, usize)>,
) {
    if last_line <= 1 {
        return;
    }

    let check_line = last_line - 1;
    let report_line = match style {
        BodyBoundaryStyle::NoEmptyLines => check_line,
        BodyBoundaryStyle::EmptyLines => last_line,
    };
    let message = match style {
        BodyBoundaryStyle::NoEmptyLines => {
            format!("Extra empty line detected at {body_kind} body end.")
        }
        BodyBoundaryStyle::EmptyLines => format!("Empty line missing at {body_kind} body end."),
    };

    check_boundary_line(
        cop_name,
        source,
        check_line,
        report_line,
        style,
        message,
        diagnostics,
        corrections,
        emitted_locations,
    );
}

#[allow(clippy::too_many_arguments)]
fn check_boundary_line(
    cop_name: &'static str,
    source: &SourceFile,
    check_line: usize,
    report_line: usize,
    style: BodyBoundaryStyle,
    message: String,
    diagnostics: &mut Vec<Diagnostic>,
    mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    emitted_locations: &mut std::collections::HashSet<(usize, usize)>,
) {
    let Some(line) = util::line_at(source, check_line) else {
        return;
    };

    let should_emit = match style {
        BodyBoundaryStyle::NoEmptyLines => util::is_blank_line(line),
        BodyBoundaryStyle::EmptyLines => !util::is_blank_line(line),
    };
    if !should_emit {
        return;
    }

    let location = (report_line, 0);
    if !emitted_locations.insert(location) {
        return;
    }

    let mut diagnostic = Diagnostic {
        path: source.path_str().to_string(),
        location: Location {
            line: report_line,
            column: 0,
        },
        severity: Severity::Convention,
        cop_name: cop_name.to_string(),
        message,
        corrected: false,
    };

    if let Some(ref mut corr) = corrections {
        match style {
            BodyBoundaryStyle::NoEmptyLines => {
                if let (Some(start), Some(end)) = (
                    source.line_col_to_offset(report_line, 0),
                    source.line_col_to_offset(report_line + 1, 0),
                ) {
                    corr.push(crate::correction::Correction {
                        start,
                        end,
                        replacement: String::new(),
                        cop_name,
                        cop_index: 0,
                    });
                    diagnostic.corrected = true;
                }
            }
            BodyBoundaryStyle::EmptyLines => {
                if let Some(offset) = source.line_col_to_offset(report_line, 0) {
                    corr.push(crate::correction::Correction {
                        start: offset,
                        end: offset,
                        replacement: "\n".to_string(),
                        cop_name,
                        cop_index: 0,
                    });
                    diagnostic.corrected = true;
                }
            }
        }
    }

    diagnostics.push(diagnostic);
}

fn is_comment_line(line: &[u8]) -> bool {
    let mut index = 0;
    while index < line.len() && matches!(line[index], b' ' | b'\t' | b'\r' | b'\x0C' | b'\x0B') {
        index += 1;
    }
    line.get(index) == Some(&b'#')
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

/// Find the first line number of the first child that requires an empty line.
/// Returns the line number and RuboCop's message type ("def", "class",
/// "module", or "send" for bare visibility modifiers).
/// Returns None if no such child is found.
fn first_empty_line_required_child_line(
    source: &SourceFile,
    body: Option<&ruby_prism::Node<'_>>,
) -> Option<(usize, &'static str)> {
    match body {
        None => None,
        Some(node) => {
            let children: Vec<_> = if let Some(stmts) = node.as_statements_node() {
                stmts.body().iter().collect()
            } else if let Some(begin) = node.as_begin_node() {
                if let Some(stmts) = begin.statements() {
                    stmts.body().iter().collect()
                } else {
                    vec![]
                }
            } else {
                if let Some(type_name) = empty_line_required_type_name(node) {
                    let line = source.offset_to_line_col(node.location().start_offset()).0;
                    return Some((line, type_name));
                }
                return None;
            };
            for child in &children {
                if let Some(type_name) = empty_line_required_type_name(child) {
                    let line = source.offset_to_line_col(child.location().start_offset()).0;
                    return Some((line, type_name));
                }
            }
            None
        }
    }
}

/// Check for deferred empty line requirement.
/// When the first child doesn't require an empty line, but a subsequent child
/// does, RuboCop walks upward from that child, skipping comment lines only.
/// A whitespace-only line does NOT satisfy this check.
fn check_deferred_empty_line(
    cop_name: &'static str,
    source: &SourceFile,
    body: Option<&ruby_prism::Node<'_>>,
    diagnostics: &mut Vec<Diagnostic>,
    mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    emitted_locations: &mut std::collections::HashSet<(usize, usize)>,
) {
    let Some((child_line, type_name)) = first_empty_line_required_child_line(source, body) else {
        return;
    };

    let mut previous_non_comment_line = child_line.saturating_sub(1);
    while previous_non_comment_line > 0 {
        let Some(line) = util::line_at(source, previous_non_comment_line) else {
            return;
        };
        if !is_comment_line(line) {
            if util::is_blank_line(line) {
                return;
            }
            break;
        }
        previous_non_comment_line -= 1;
    }

    let report_line = previous_non_comment_line + 1;
    if !emitted_locations.insert((report_line, 0)) {
        return;
    }

    let mut diagnostic = Diagnostic {
        path: source.path_str().to_string(),
        location: Location {
            line: report_line,
            column: 0,
        },
        severity: Severity::Convention,
        cop_name: cop_name.to_string(),
        message: format!("Empty line missing before first {} definition.", type_name),
        corrected: false,
    };

    if let Some(ref mut corr) = corrections {
        if let Some(offset) = source.line_col_to_offset(report_line, 0) {
            corr.push(crate::correction::Correction {
                start: offset,
                end: offset,
                replacement: "\n".to_string(),
                cop_name,
                cop_index: 0,
            });
            diagnostic.corrected = true;
        }
    }

    diagnostics.push(diagnostic);
}

/// Check if the first child of the body requires an empty line before it.
/// Per RuboCop: `{any_def class module (send nil? {:private :protected :public})}`
fn first_child_requires_empty_line(body: &Option<ruby_prism::Node<'_>>) -> bool {
    match body {
        None => false,
        Some(node) => {
            if let Some(stmts) = node.as_statements_node() {
                let children: Vec<_> = stmts.body().iter().collect();
                if children.is_empty() {
                    return false;
                }
                let first = &children[0];
                empty_line_required_type_name(first).is_some()
            } else if let Some(begin) = node.as_begin_node() {
                if let Some(stmts) = begin.statements() {
                    let children: Vec<_> = stmts.body().iter().collect();
                    if children.is_empty() {
                        return false;
                    }
                    let first = &children[0];
                    empty_line_required_type_name(first).is_some()
                } else {
                    false
                }
            } else {
                empty_line_required_type_name(node).is_some()
            }
        }
    }
}

fn empty_line_required_type_name(node: &ruby_prism::Node<'_>) -> Option<&'static str> {
    if node.as_def_node().is_some() {
        Some("def")
    } else if node.as_class_node().is_some() {
        Some("class")
    } else if node.as_module_node().is_some() {
        Some("module")
    } else if is_bare_access_modifier(node) {
        Some("send")
    } else {
        None
    }
}

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

/// Check if a node is a def, class, or module node.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{run_cop_full, run_cop_full_with_config};

    crate::cop_fixture_tests!(
        EmptyLinesAroundClassBody,
        "cops/layout/empty_lines_around_class_body"
    );
    crate::cop_autocorrect_fixture_tests!(
        EmptyLinesAroundClassBody,
        "cops/layout/empty_lines_around_class_body"
    );

    fn style_config(style: &str) -> CopConfig {
        use std::collections::HashMap;

        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String(style.into()),
            )]),
            ..CopConfig::default()
        }
    }

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
        let config = style_config("empty_lines");
        let src = b"class Foo\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundClassBody, src, config);
        assert_eq!(
            diags.len(),
            2,
            "empty_lines style should require blank lines at both ends"
        );
    }

    #[test]
    fn empty_lines_ignores_comment_only_body() {
        let src = b"class Base\n  # docs only\n  # still empty\nend\n";
        let diags =
            run_cop_full_with_config(&EmptyLinesAroundClassBody, src, style_config("empty_lines"));
        assert!(
            diags.is_empty(),
            "comment-only body should be ignored for empty_lines, got: {:?}",
            diags
        );
    }

    #[test]
    fn empty_lines_except_namespace_ignores_comment_only_body() {
        let src = b"class Base\n  # docs only\n  # still empty\nend\n";
        let diags = run_cop_full_with_config(
            &EmptyLinesAroundClassBody,
            src,
            style_config("empty_lines_except_namespace"),
        );
        assert!(
            diags.is_empty(),
            "comment-only body should be ignored for empty_lines_except_namespace, got: {:?}",
            diags
        );
    }

    #[test]
    fn empty_lines_two_line_body_reports_beginning_only() {
        let src = b"class Foo; bar\nend\n";
        let diags =
            run_cop_full_with_config(&EmptyLinesAroundClassBody, src, style_config("empty_lines"));
        assert_eq!(diags.len(), 1, "2-line body should collapse to one offense");
        assert_eq!(diags[0].location.line, 2);
        assert_eq!(
            diags[0].message,
            "Empty line missing at class body beginning."
        );
    }

    #[test]
    fn empty_lines_except_namespace_two_line_body_reports_beginning_only() {
        let src = b"class Foo; bar\nend\n";
        let diags = run_cop_full_with_config(
            &EmptyLinesAroundClassBody,
            src,
            style_config("empty_lines_except_namespace"),
        );
        assert_eq!(diags.len(), 1, "2-line body should collapse to one offense");
        assert_eq!(diags[0].location.line, 2);
        assert_eq!(
            diags[0].message,
            "Empty line missing at class body beginning."
        );
    }

    #[test]
    fn beginning_only_style() {
        let config = style_config("beginning_only");
        // No blank at beginning => flag missing beginning blank
        let src = b"class Foo\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundClassBody, src, config);
        assert!(
            diags.iter().any(|d| d.message.contains("beginning")),
            "beginning_only should flag missing blank at beginning"
        );
    }

    #[test]
    fn empty_lines_special_with_def_first_child() {
        // When first child is def, empty_lines_special requires empty lines at BOTH ends
        let config = style_config("empty_lines_special");
        let src = b"class Foo\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundClassBody, src, config);
        assert_eq!(
            diags.len(),
            2,
            "empty_lines_special with def first child should require blank lines at both ends"
        );
        assert!(
            diags.iter().any(|d| d.message.contains("beginning")),
            "should flag missing blank at beginning"
        );
        assert!(
            diags.iter().any(|d| d.message.contains("end")),
            "should flag missing blank at end"
        );
    }

    #[test]
    fn empty_lines_special_with_include_first_child() {
        // When first child is NOT def/class/module, empty_lines_special:
        // - No blank required at beginning
        // - Deferred: blank required before first def
        // - Blank required at end
        let config = style_config("empty_lines_special");
        // No blank at beginning, no blank before def, no blank at end
        let src = b"class Foo\n  include Something\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundClassBody, src, config);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("first def definition")),
            "should flag missing blank before first def (deferred check)"
        );
        assert!(
            diags.iter().any(|d| d.message.contains("end")),
            "should flag missing blank at end"
        );
        // Should NOT flag beginning
        assert!(
            !diags.iter().any(|d| d.message.contains("beginning")),
            "should NOT flag missing blank at beginning"
        );
    }

    #[test]
    fn empty_lines_special_blank_before_def_no_offense() {
        // When first child is not def/class/module and a blank line EXISTS before
        // the first def, the deferred check should NOT fire.
        let config = style_config("empty_lines_special");
        let src = b"class Foo\n  include Something\n\n  def bar; end\n\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundClassBody, src, config);
        assert!(
            !diags.iter().any(|d| d.message.contains("first def")),
            "blank line before def exists — deferred check should not fire, got: {:?}",
            diags
        );
    }

    #[test]
    fn empty_lines_special_ignores_comments_before_first_def() {
        let src =
            b"class Foo\n  include Something\n\n  # docs\n  # more docs\n  def bar\n  end\n\nend\n";
        let diags = run_cop_full_with_config(
            &EmptyLinesAroundClassBody,
            src,
            style_config("empty_lines_special"),
        );
        assert!(
            !diags.iter().any(|d| d.message.contains("first def")),
            "comment lines after a blank separator should not trigger the deferred check, got: {:?}",
            diags
        );
    }

    #[test]
    fn empty_lines_special_reports_missing_before_commented_def_on_comment_line() {
        let src = b"class Foo\n  include Something\n  # docs\n  def bar\n  end\n\nend\n";
        let diags = run_cop_full_with_config(
            &EmptyLinesAroundClassBody,
            src,
            style_config("empty_lines_special"),
        );
        assert!(
            diags
                .iter()
                .any(|d| d.location.line == 3 && d.message.contains("first def definition")),
            "missing separator before commented def should report on the first comment line, got: {:?}",
            diags
        );
    }

    #[test]
    fn empty_lines_special_whitespace_before_def_reports_on_def_line() {
        let src = b"class Foo\n  attr_accessor :project\n  attr_accessor :token\n  \n  def initialize(project, token)\n  end\n\nend\n";
        let diags = run_cop_full_with_config(
            &EmptyLinesAroundClassBody,
            src,
            style_config("empty_lines_special"),
        );
        assert!(
            diags
                .iter()
                .any(|d| d.location.line == 5 && d.message.contains("first def definition")),
            "whitespace-only separator should still offend and report on the def line, got: {:?}",
            diags
        );
    }

    #[test]
    fn empty_lines_special_reports_private_as_send() {
        let src = b"class Foo\n  include Something\n  private\n  def bar\n  end\n\nend\n";
        let diags = run_cop_full_with_config(
            &EmptyLinesAroundClassBody,
            src,
            style_config("empty_lines_special"),
        );
        assert!(
            diags
                .iter()
                .any(|d| d.location.line == 3 && d.message.contains("first send definition")),
            "bare visibility modifiers should match RuboCop's deferred `send` message, got: {:?}",
            diags
        );
    }

    #[test]
    fn empty_lines_special_namespace_no_empty_lines() {
        // Namespace (single direct class/module child) uses no_empty_lines style
        let config = style_config("empty_lines_special");
        // Single class child - should use no_empty_lines (no blank lines)
        let src = b"class Parent\n  class Child\n  end\nend\n";
        let diags = run_cop_full_with_config(&EmptyLinesAroundClassBody, src, config);
        assert!(
            diags.is_empty(),
            "empty_lines_special namespace should use no_empty_lines (no offenses), got: {:?}",
            diags
        );
    }
}
