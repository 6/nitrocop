use ruby_prism::Visit;

use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// FN fix #1: The `inside_class_or_module` boolean was too broad — it suppressed
/// detection of ALL compact-style definitions nested inside any class/module.
/// Changed to `parent_is_class_or_module` matching RuboCop's single-statement
/// body semantics. This resolved ~636 FN.
///
/// FN fix #2: The `has_cbase` function walked the entire constant path chain,
/// returning true for `::Foo::Bar` (multi-segment cbase). But RuboCop's
/// `node.identifier.namespace&.cbase_type?` only skips when the immediate
/// namespace is cbase — i.e., `::Foo` but NOT `::Foo::Bar`. Changed to
/// `is_namespace_cbase` which only checks the direct parent, resolving ~217 FN.
///
/// FN fix #3: A compact-style class inside an `if` nested under a single-statement
/// class/module body (for example `module A; if cond; class B::C; end; end; end`)
/// was missed because `parent_is_class_or_module` leaked through the conditional.
/// Reset that state for `if`/`unless`, matching RuboCop's direct-parent check.
///
/// FP fix: RuboCop crashes on expression-based class/module defs
/// (`x = module Foo::Bar`, `@var = class Foo::Bar < Base`), producing
/// 0 offenses. Skip class/module nodes that are direct values of variable
/// assignments to match the observable behavior.
///
/// Variant fix #1 (superclass check): RuboCop's `on_class` early-returns
/// `if node.parent_class && style != :nested` where `style` is the global
/// `EnforcedStyle` (from `ConfigurableEnforcedStyle`), NOT `style_for_classes`.
/// Our code was using the effective per-class style, which differs when
/// `EnforcedStyleForClasses` overrides `EnforcedStyle`. Fixed to use
/// `enforced_style` for the superclass check.
///
/// Variant fix #2 (string/symbol mismatch): In RuboCop, `style_for_classes`
/// and `style_for_modules` return a Ruby String when `EnforcedStyleForClasses`
/// / `EnforcedStyleForModules` is explicitly set, but `check_style` compares
/// with a Symbol (`:nested`). In Ruby, `"nested" != :nested`, so the
/// comparison always fails and falls through to `check_compact_style`.
/// We replicate this: when the override is set, always use compact checking.
pub struct ClassAndModuleChildren;

impl Cop for ClassAndModuleChildren {
    fn name(&self) -> &'static str {
        "Style/ClassAndModuleChildren"
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
        let enforced_style = config.get_str("EnforcedStyle", "nested").to_string();
        let enforced_for_classes = config.get_str("EnforcedStyleForClasses", "").to_string();
        let enforced_for_modules = config.get_str("EnforcedStyleForModules", "").to_string();

        let mut visitor = ChildrenVisitor {
            source,
            enforced_style,
            enforced_for_classes,
            enforced_for_modules,
            parent_is_class_or_module: false,
            skip_next_class_or_module: false,
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }

    fn diagnostic(
        &self,
        source: &SourceFile,
        line: usize,
        column: usize,
        message: String,
    ) -> Diagnostic {
        Diagnostic {
            path: source.path_str().to_string(),
            location: crate::diagnostic::Location { line, column },
            severity: self.default_severity(),
            cop_name: self.name().to_string(),
            message,
            corrected: false,
        }
    }
}

struct ChildrenVisitor<'a> {
    source: &'a SourceFile,
    enforced_style: String,
    enforced_for_classes: String,
    enforced_for_modules: String,
    /// Mirrors RuboCop's `node.parent&.type?(:class, :module)`.
    /// True when the current node is the sole body statement of a class/module,
    /// meaning its AST parent (in parser gem terms) IS the class/module itself.
    /// When a class/module body has multiple statements, parser gem wraps them
    /// in a `begin` node, so children's parent is `begin`, not the class/module.
    parent_is_class_or_module: bool,
    /// True when the next class/module node is a direct value of a variable
    /// assignment (e.g., `x = class Foo::Bar; end`). RuboCop crashes on these
    /// patterns, producing 0 offenses. We skip them to match observable behavior.
    skip_next_class_or_module: bool,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> ChildrenVisitor<'a> {
    fn add_diagnostic(&mut self, offset: usize, message: String) {
        let (line, column) = self.source.offset_to_line_col(offset);
        self.diagnostics.push(Diagnostic {
            path: self.source.path_str().to_string(),
            location: crate::diagnostic::Location { line, column },
            severity: crate::diagnostic::Severity::Convention,
            cop_name: "Style/ClassAndModuleChildren".to_string(),
            message,
            corrected: false,
        });
    }

    /// Check if the body of a class/module is a single class or module definition
    /// that could be compacted. In Prism, the body is either a StatementsNode
    /// containing a single child, or None.
    fn body_is_single_class_or_module(&self, body: &Option<ruby_prism::Node<'a>>) -> bool {
        let Some(body_node) = body else {
            return false;
        };
        // The body is typically a StatementsNode wrapping one or more statements
        if let Some(stmts) = body_node.as_statements_node() {
            let children: Vec<_> = stmts.body().iter().collect();
            if children.len() == 1 {
                let child = &children[0];
                return child.as_class_node().is_some() || child.as_module_node().is_some();
            }
        }
        // If the body is directly a class or module (shouldn't normally happen but handle it)
        body_node.as_class_node().is_some() || body_node.as_module_node().is_some()
    }

    fn check_nested_style(&mut self, is_compact: bool, name_offset: usize) {
        // For nested style: flag compact-style definitions (with ::)
        if !is_compact {
            return;
        }
        // RuboCop: return if node.parent&.type?(:class, :module)
        // Only skip when this node is the sole body statement of a parent class/module.
        if self.parent_is_class_or_module {
            return;
        }
        self.add_diagnostic(
            name_offset,
            "Use nested module/class definitions instead of compact style.".to_string(),
        );
    }

    fn check_compact_style(&mut self, body: &Option<ruby_prism::Node<'a>>, name_offset: usize) {
        // For compact style: flag outer nodes whose body is a single class/module
        // RuboCop: return if parent&.type?(:class, :module)
        if self.parent_is_class_or_module {
            return;
        }
        if !self.body_is_single_class_or_module(body) {
            return;
        }
        self.add_diagnostic(
            name_offset,
            "Use compact module/class definition instead of nested style.".to_string(),
        );
    }
}

/// Count the number of statements in a class/module body.
/// In RuboCop's parser gem, single-statement bodies make the child's parent
/// the class/module itself, while multi-statement bodies wrap in a `begin` node.
fn body_statement_count(body: &Option<ruby_prism::Node<'_>>) -> usize {
    let Some(body_node) = body else {
        return 0;
    };
    if let Some(stmts) = body_node.as_statements_node() {
        stmts.body().iter().count()
    } else {
        1
    }
}

/// Check if a constant path's immediate namespace is cbase (the `::` prefix).
/// Matches RuboCop's `node.identifier.namespace&.cbase_type?`.
/// Returns true only for `::Foo` (namespace is cbase), NOT for `::Foo::Bar`
/// (namespace is `::Foo`, which is a const node, not cbase).
fn is_namespace_cbase(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(cp) = node.as_constant_path_node() {
        // parent() is None means the namespace is cbase (::)
        // For ::Foo, parent is None → true
        // For ::Foo::Bar, parent is ConstantPathNode(::Foo) → false
        cp.parent().is_none()
    } else {
        false
    }
}

/// Check if a node is a class or module node (used by write-node visitors).
fn is_class_or_module_node(node: &ruby_prism::Node<'_>) -> bool {
    node.as_class_node().is_some() || node.as_module_node().is_some()
}

impl<'a> Visit<'a> for ChildrenVisitor<'a> {
    // Skip class/module definitions used as assignment values.
    // RuboCop crashes on `x = class Foo::Bar; end` patterns, producing 0 offenses.
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'a>) {
        if is_class_or_module_node(&node.value()) {
            self.skip_next_class_or_module = true;
        }
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_instance_variable_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableWriteNode<'a>,
    ) {
        if is_class_or_module_node(&node.value()) {
            self.skip_next_class_or_module = true;
        }
        ruby_prism::visit_instance_variable_write_node(self, node);
    }

    fn visit_class_variable_write_node(&mut self, node: &ruby_prism::ClassVariableWriteNode<'a>) {
        if is_class_or_module_node(&node.value()) {
            self.skip_next_class_or_module = true;
        }
        ruby_prism::visit_class_variable_write_node(self, node);
    }

    fn visit_global_variable_write_node(&mut self, node: &ruby_prism::GlobalVariableWriteNode<'a>) {
        if is_class_or_module_node(&node.value()) {
            self.skip_next_class_or_module = true;
        }
        ruby_prism::visit_global_variable_write_node(self, node);
    }

    // Reset parent_is_class_or_module inside wrappers whose children do not have
    // the enclosing class/module as their direct AST parent in RuboCop.
    // In RuboCop, node.parent is the direct AST parent. A class inside a block
    // (e.g., `before do; class Foo::Bar; end; end`) has a block/begin parent,
    // not a class/module parent. Without this reset, the flag from an enclosing
    // single-statement module body would leak through blocks.
    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'a>) {
        let prev = self.parent_is_class_or_module;
        self.parent_is_class_or_module = false;
        ruby_prism::visit_block_node(self, node);
        self.parent_is_class_or_module = prev;
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'a>) {
        let prev = self.parent_is_class_or_module;
        self.parent_is_class_or_module = false;
        ruby_prism::visit_def_node(self, node);
        self.parent_is_class_or_module = prev;
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'a>) {
        let prev = self.parent_is_class_or_module;
        self.parent_is_class_or_module = false;
        ruby_prism::visit_if_node(self, node);
        self.parent_is_class_or_module = prev;
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'a>) {
        let prev = self.parent_is_class_or_module;
        self.parent_is_class_or_module = false;
        ruby_prism::visit_unless_node(self, node);
        self.parent_is_class_or_module = prev;
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'a>) {
        // Skip expression-based class definitions (RuboCop crashes on these)
        let skip = self.skip_next_class_or_module;
        self.skip_next_class_or_module = false;
        if skip {
            let prev = self.parent_is_class_or_module;
            self.parent_is_class_or_module = body_statement_count(&node.body()) == 1;
            ruby_prism::visit_class_node(self, node);
            self.parent_is_class_or_module = prev;
            return;
        }

        let constant_path = node.constant_path();
        let is_compact = constant_path.as_constant_path_node().is_some();
        let name_offset = constant_path.location().start_offset();

        // RuboCop: return if node.identifier.namespace&.cbase_type?
        // Skip single-name cbase paths (e.g., ::Foo) but NOT multi-segment (::Foo::Bar)
        if is_namespace_cbase(&constant_path) {
            let prev = self.parent_is_class_or_module;
            self.parent_is_class_or_module = body_statement_count(&node.body()) == 1;
            ruby_prism::visit_class_node(self, node);
            self.parent_is_class_or_module = prev;
            return;
        }

        // RuboCop: return if node.parent_class && style != :nested
        // `style` in RuboCop is the global EnforcedStyle (from ConfigurableEnforcedStyle),
        // NOT style_for_classes. Must use enforced_style here.
        let has_superclass = node.superclass().is_some();
        if has_superclass && self.enforced_style != "nested" {
            // Still visit children
            let prev = self.parent_is_class_or_module;
            self.parent_is_class_or_module = body_statement_count(&node.body()) == 1;
            ruby_prism::visit_class_node(self, node);
            self.parent_is_class_or_module = prev;
            return;
        }

        // RuboCop bug: style_for_classes returns a Ruby String when
        // EnforcedStyleForClasses is explicitly set, but check_style compares
        // with a Symbol (:nested). In Ruby, "nested" != :nested, so the
        // comparison always fails and falls through to check_compact_style.
        // We replicate this: when the override is set, always use compact.
        if !self.enforced_for_classes.is_empty() {
            let body = node.body();
            self.check_compact_style(&body, name_offset);
        } else if self.enforced_style == "nested" {
            self.check_nested_style(is_compact, name_offset);
        } else if self.enforced_style == "compact" {
            let body = node.body();
            self.check_compact_style(&body, name_offset);
        }

        // Visit children: set parent_is_class_or_module based on body count
        let prev = self.parent_is_class_or_module;
        self.parent_is_class_or_module = body_statement_count(&node.body()) == 1;
        ruby_prism::visit_class_node(self, node);
        self.parent_is_class_or_module = prev;
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'a>) {
        // Skip expression-based module definitions (RuboCop crashes on these)
        let skip = self.skip_next_class_or_module;
        self.skip_next_class_or_module = false;
        if skip {
            let prev = self.parent_is_class_or_module;
            self.parent_is_class_or_module = body_statement_count(&node.body()) == 1;
            ruby_prism::visit_module_node(self, node);
            self.parent_is_class_or_module = prev;
            return;
        }

        let constant_path = node.constant_path();
        let is_compact = constant_path.as_constant_path_node().is_some();
        let name_offset = constant_path.location().start_offset();

        // RuboCop: return if node.identifier.namespace&.cbase_type?
        if is_namespace_cbase(&constant_path) {
            let prev = self.parent_is_class_or_module;
            self.parent_is_class_or_module = body_statement_count(&node.body()) == 1;
            ruby_prism::visit_module_node(self, node);
            self.parent_is_class_or_module = prev;
            return;
        }

        // Same RuboCop string/symbol mismatch: when EnforcedStyleForModules
        // is explicitly set, it returns a String that never matches :nested,
        // so check_compact_style is always used.
        if !self.enforced_for_modules.is_empty() {
            let body = node.body();
            self.check_compact_style(&body, name_offset);
        } else if self.enforced_style == "nested" {
            self.check_nested_style(is_compact, name_offset);
        } else if self.enforced_style == "compact" {
            let body = node.body();
            self.check_compact_style(&body, name_offset);
        }

        // Visit children: set parent_is_class_or_module based on body count
        let prev = self.parent_is_class_or_module;
        self.parent_is_class_or_module = body_statement_count(&node.body()) == 1;
        ruby_prism::visit_module_node(self, node);
        self.parent_is_class_or_module = prev;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        ClassAndModuleChildren,
        "cops/style/class_and_module_children"
    );

    #[test]
    fn config_compact_style_only_flags_nested() {
        use crate::testutil::{assert_cop_no_offenses_full_with_config, run_cop_full_with_config};
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("compact".into()),
            )]),
            ..CopConfig::default()
        };
        // Top-level class with no children — should NOT trigger
        let source = b"class Foo\nend\n";
        assert_cop_no_offenses_full_with_config(&ClassAndModuleChildren, source, config.clone());

        // Module wrapping a single class — SHOULD trigger (on the module)
        let source2 = b"module A\n  class Foo\n  end\nend\n";
        let diags = run_cop_full_with_config(&ClassAndModuleChildren, source2, config.clone());
        assert_eq!(
            diags.len(),
            1,
            "Should fire for module wrapping a single class"
        );
        assert!(diags[0].message.contains("compact"));

        // Compact style class should be clean
        let source3 = b"class Foo::Bar\nend\n";
        assert_cop_no_offenses_full_with_config(&ClassAndModuleChildren, source3, config.clone());

        // Class wrapping a single class — should NOT trigger (inside_class_or_module
        // is not the issue; the outer class has a child class but classes with children
        // still get checked. However, the outer class has a superclass? No. Let's verify.)
        let source4 = b"class A\n  class Foo\n  end\nend\n";
        let diags4 = run_cop_full_with_config(&ClassAndModuleChildren, source4, config.clone());
        // RuboCop DOES flag this: outer class wraps a single class child.
        // But wait -- does it? Let me check: on_class returns early if parent_class && style != :nested.
        // class A has no parent_class (superclass), so it proceeds to check_compact_style.
        // The body is a single class, so it flags it.
        assert_eq!(
            diags4.len(),
            1,
            "Module wrapping single class should be flagged"
        );

        // Class with superclass wrapping a class — should NOT trigger
        // (on_class returns early: node.parent_class && style != :nested)
        let source5 = b"class A < Base\n  class Foo\n  end\nend\n";
        assert_cop_no_offenses_full_with_config(&ClassAndModuleChildren, source5, config);
    }

    #[test]
    fn top_level_module_no_offense_with_compact() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("compact".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"module Foo\nend\n";
        assert_cop_no_offenses_full_with_config(&ClassAndModuleChildren, source, config);
    }

    #[test]
    fn compact_style_class_inside_class_with_superclass_no_offense() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("compact".into()),
            )]),
            ..CopConfig::default()
        };
        // Class with superclass wrapping a child class — RuboCop skips this because
        // on_class returns early when parent_class is present and style != :nested.
        // This is the chatwoot pattern (e.g., class InboxPolicy < ApplicationPolicy; class Scope; end; end)
        let source = b"class InboxPolicy < ApplicationPolicy\n  class Scope\n    def resolve\n      super\n    end\n  end\nend\n";
        assert_cop_no_offenses_full_with_config(&ClassAndModuleChildren, source, config.clone());

        // Module wrapping multiple classes — should NOT flag (body is not a single class)
        let source2 = b"module CustomExceptions::Account\n  class InvalidEmail < Base\n    def message; end\n  end\n  class UserExists < Base\n    def message; end\n  end\nend\n";
        assert_cop_no_offenses_full_with_config(&ClassAndModuleChildren, source2, config.clone());

        // Module wrapping a single class — SHOULD flag
        let source3 = b"module Api\n  class SessionsController\n  end\nend\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&ClassAndModuleChildren, source3, config);
        assert_eq!(
            diags.len(),
            1,
            "Module wrapping single class should be flagged with compact style"
        );
    }

    #[test]
    fn compact_style_nested_inside_other_class_module_not_flagged() {
        use crate::testutil::{assert_cop_no_offenses_full_with_config, run_cop_full_with_config};
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("compact".into()),
            )]),
            ..CopConfig::default()
        };
        // Class (no superclass) wrapping module — RuboCop DOES flag this (body is single module)
        let source = b"class Foo\n  module Bar\n    class Baz\n    end\n  end\nend\n";
        let diags = run_cop_full_with_config(&ClassAndModuleChildren, source, config.clone());
        assert_eq!(
            diags.len(),
            1,
            "Class wrapping single module should be flagged"
        );

        // But the inner module (Bar wrapping Baz) should NOT be flagged separately
        // because Bar is inside a class/module (Foo). Only the outermost is flagged.
        assert!(
            diags[0].location.line == 1,
            "Only the outer class should be flagged"
        );

        // Class with superclass wrapping module — should NOT be flagged
        let source2 = b"class Foo < Base\n  module Bar\n    class Baz\n    end\n  end\nend\n";
        assert_cop_no_offenses_full_with_config(&ClassAndModuleChildren, source2, config);
    }

    #[test]
    fn enforced_style_for_classes_overrides() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("nested".into()),
                ),
                (
                    "EnforcedStyleForClasses".into(),
                    serde_yml::Value::String("compact".into()),
                ),
            ]),
            ..CopConfig::default()
        };
        // Top-level class wrapping a single class — should be flagged (compact for classes)
        let source = b"class A\n  class Foo\n  end\nend\n";
        let diags = run_cop_full_with_config(&ClassAndModuleChildren, source, config.clone());
        assert_eq!(diags.len(), 1, "Class should be flagged with compact style");
        assert!(diags[0].message.contains("compact"));

        // Module should still use nested style
        let source2 = b"module Foo::Bar\nend\n";
        let diags2 = run_cop_full_with_config(&ClassAndModuleChildren, source2, config);
        assert_eq!(
            diags2.len(),
            1,
            "Module should be flagged with nested style"
        );
        assert!(diags2[0].message.contains("nested"));
    }

    /// Variant batch 1: EnforcedStyle=compact, ForClasses=nested, ForModules=nested.
    /// In RuboCop, style_for_classes returns the STRING "nested" which fails
    /// the symbol comparison `style == :nested`, so check_compact_style is used.
    /// Also, superclass check uses the global EnforcedStyle (compact), so classes
    /// with superclass are skipped.
    #[test]
    fn variant_compact_for_classes_nested_for_modules_nested() {
        use crate::testutil::{assert_cop_no_offenses_full_with_config, run_cop_full_with_config};
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("compact".into()),
                ),
                (
                    "EnforcedStyleForClasses".into(),
                    serde_yml::Value::String("nested".into()),
                ),
                (
                    "EnforcedStyleForModules".into(),
                    serde_yml::Value::String("nested".into()),
                ),
            ]),
            ..CopConfig::default()
        };

        // Compact-style class should NOT be flagged (check_compact_style only looks
        // at nestable bodies, not compact names)
        let source = b"class Foo::Bar\nend\n";
        assert_cop_no_offenses_full_with_config(&ClassAndModuleChildren, source, config.clone());

        // Compact-style module should NOT be flagged
        let source2 = b"module Baz::Qux\nend\n";
        assert_cop_no_offenses_full_with_config(&ClassAndModuleChildren, source2, config.clone());

        // Nested class wrapping single child — SHOULD be flagged (compact check)
        let source3 = b"class A\n  class B\n  end\nend\n";
        let diags = run_cop_full_with_config(&ClassAndModuleChildren, source3, config.clone());
        assert_eq!(
            diags.len(),
            1,
            "Class wrapping single class should be flagged"
        );
        assert!(diags[0].message.contains("compact"));

        // Nested module wrapping single child — SHOULD be flagged (compact check)
        let source4 = b"module C\n  module D\n  end\nend\n";
        let diags2 = run_cop_full_with_config(&ClassAndModuleChildren, source4, config.clone());
        assert_eq!(
            diags2.len(),
            1,
            "Module wrapping single module should be flagged"
        );
        assert!(diags2[0].message.contains("compact"));

        // Class with superclass wrapping child — NOT flagged (EnforcedStyle=compact,
        // so on_class returns early when superclass present)
        let source5 = b"class E < Base\n  class F\n  end\nend\n";
        assert_cop_no_offenses_full_with_config(&ClassAndModuleChildren, source5, config);
    }

    /// Variant batch 2: EnforcedStyle=nested(default), ForClasses=compact, ForModules=compact.
    /// The superclass check uses EnforcedStyle (nested), so classes with superclass
    /// are NOT skipped. The style override makes both use check_compact_style.
    #[test]
    fn variant_nested_for_classes_compact_for_modules_compact() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyleForClasses".into(),
                    serde_yml::Value::String("compact".into()),
                ),
                (
                    "EnforcedStyleForModules".into(),
                    serde_yml::Value::String("compact".into()),
                ),
            ]),
            ..CopConfig::default()
        };

        // Class with superclass wrapping child — SHOULD be flagged
        // (EnforcedStyle=nested, so superclass does NOT trigger early return)
        let source = b"class E < Base\n  class F\n  end\nend\n";
        let diags = run_cop_full_with_config(&ClassAndModuleChildren, source, config.clone());
        assert_eq!(
            diags.len(),
            1,
            "Class with superclass wrapping single child should be flagged"
        );
        assert!(diags[0].message.contains("compact"));

        // Nested class wrapping single child — SHOULD be flagged
        let source2 = b"class A\n  class B\n  end\nend\n";
        let diags2 = run_cop_full_with_config(&ClassAndModuleChildren, source2, config);
        assert_eq!(
            diags2.len(),
            1,
            "Class wrapping single class should be flagged"
        );
        assert!(diags2[0].message.contains("compact"));
    }

    #[test]
    fn enforced_style_for_modules_overrides() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("nested".into()),
                ),
                (
                    "EnforcedStyleForModules".into(),
                    serde_yml::Value::String("compact".into()),
                ),
            ]),
            ..CopConfig::default()
        };
        // Module wrapping a single module — should be flagged (compact for modules)
        let source = b"module A\n  module Foo\n  end\nend\n";
        let diags = run_cop_full_with_config(&ClassAndModuleChildren, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Module should be flagged with compact style"
        );
        assert!(diags[0].message.contains("compact"));
    }
}
