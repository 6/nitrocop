use crate::cop::shared::access_modifier_predicates;
use crate::cop::shared::node_type::{
    BLOCK_NODE, CALL_NODE, CLASS_NODE, MODULE_NODE, SINGLETON_CLASS_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// ## Corpus investigation (2026-03-10)
///
/// Cached corpus oracle reported FP=11, FN=2.
///
/// ### Round 1 (2026-03-10): IndentationWidth support
/// Fixed: this cop was already walking block bodies, but it still hardcoded a
/// 2-space indent for `EnforcedStyle: indent` and ignored `Layout/IndentationWidth`.
/// That produced false positives in width-4 repos and missed corresponding
/// under-indented modifiers in the same configs.
///
/// ### Round 2 (2026-03-14): Use `end` keyword column instead of opening line indentation
/// Root cause of remaining 11 FPs: the cop computed expected indentation from the
/// indentation of the line containing the opening keyword (`class`, `do`, etc.).
/// RuboCop instead measures the column offset between the access modifier and the
/// `end` keyword (or `}` for brace blocks). These differ when the opening keyword
/// is not at the start of its line (e.g., `Post = Struct.new(...) do` where `do`
/// is far right but `end` is aligned with `Post`). Also handles `Module.new do`
/// blocks where `end` is deeply indented. FN pattern: `private` at wrong column
/// relative to `end` keyword (e.g., col 4 in a class whose `end` is at col 0,
/// expecting col 2).
///
/// ### Round 3 (2026-03-31): Skip single-statement bodies
/// Root cause of remaining 2 FPs: RuboCop only checks access modifier indentation
/// when the body is `begin_type?` (i.e., 2+ statements). A body containing only
/// an access modifier and no other statements is not flagged. This matches cases
/// like `class Foo\n  protected\nend` or `module ClassMethods\n  private\nend`.
///
/// ### Round 4 (2026-04-08): Skip blocks inside method definitions (macro scope)
/// Root cause of FPs with `EnforcedStyle: outdent`: RuboCop's
/// `bare_access_modifier?` calls `macro?` → `in_macro_scope?`, which walks the
/// parent chain. A block inside a `def`/`defs` is NOT in macro scope (unless a
/// class/module/sclass or `class_constructor?` resets scope between the def and
/// the block). `class_constructor?` matches `Class.new`, `Module.new`,
/// `Struct.new`, and `Data.define` — these act as class-like scope boundaries.
/// Access modifiers inside non-class-constructor blocks nested inside methods
/// are not considered "bare" access modifiers by RuboCop, so the cop skips them.
/// Fixed by adding a `MacroScopeChecker` visitor that walks the AST from the root
/// to the block node, tracking `in_def` state. Blocks not in macro scope are now
/// skipped. Class constructor blocks reset `in_def` like class/module/sclass.
///
/// ### Round 5 (2026-04-11): Constant assignment (casgn) breaks macro scope
/// Root cause of 80 FPs with `EnforcedStyle: outdent`: constant assignments
/// (`FOO = SomeBuilder.new do...end`) break the `in_macro_scope?` chain in
/// RuboCop. In parser's AST, `casgn` is not a transparent wrapper (not one of
/// kwbegin/begin/any_block/if-body), so access modifiers inside a block wrapped
/// in a constant assignment are not in macro scope — unless the call is a
/// `class_constructor?` (Class.new/Module.new/Struct.new/Data.define), which
/// resets scope. Corpus pattern: `cyberark/conjur` uses `CommandClass.new(...)`
/// (not a class constructor) extensively with constant assignment. Fixed by
/// adding `visit_constant_write_node` and `visit_constant_path_write_node` to
/// `MacroScopeChecker`, which set `in_def = true` (breaking macro scope) when
/// entering constant assignments.
///
/// ### Round 6 (2026-04-11): Other non-transparent parents also break macro scope
/// Root cause of the remaining 5 FPs with `EnforcedStyle: outdent`: the first
/// macro-scope checker only tracked `def` and constant assignment wrappers. The
/// remaining corpus examples sat inside other parents that RuboCop does NOT
/// treat as transparent for `in_macro_scope?`, notably local assignments
/// (`app = Sinatra.new do ... end`) and rescuing `begin` bodies. That let
/// nested arbitrary blocks inherit macro scope when RuboCop would stop at the
/// assignment or `begin .. rescue` wrapper. Fixed by replacing the boolean
/// `in_def` tracker with a small macro-scope stack that mirrors RuboCop's
/// transparent wrappers vs non-transparent parents for the cases seen here,
/// while still letting `Class.new`/`Module.new`/`Struct.new`/`Data.define`
/// reset scope like real class/module bodies.
pub struct AccessModifierIndentation;

// Uses access_modifier_predicates::is_bare_access_modifier() instead of local constant.

fn body_statements(body: ruby_prism::Node<'_>) -> Vec<ruby_prism::Node<'_>> {
    if let Some(stmts) = body.as_statements_node() {
        stmts.body().iter().collect()
    } else {
        vec![body]
    }
}

/// Checks if a block node (identified by offset) is in macro scope.
///
/// RuboCop's `bare_access_modifier?` calls `macro?` → `in_macro_scope?`, which
/// walks the parent chain. A block inside a `def`/`defs` is NOT in macro scope
/// (unless a class/module/sclass/class_constructor resets scope between the def
/// and the block). `class_constructor?` matches `Class.new`, `Module.new`,
/// `Struct.new`, and `Data.define` — these act like class/module nodes for scope.
/// This means access modifiers inside such blocks are not "bare" access modifiers
/// and should not be checked by this cop.
struct MacroScopeChecker {
    target_start: usize,
    target_end: usize,
    macro_scope_stack: Vec<access_modifier_predicates::MacroScope>,
    result: Option<bool>,
}

impl MacroScopeChecker {
    fn new(target_start: usize, target_end: usize) -> Self {
        Self {
            target_start,
            target_end,
            macro_scope_stack: vec![],
            result: None,
        }
    }

    fn current_macro_scope(&self) -> bool {
        access_modifier_predicates::in_macro_scope(&self.macro_scope_stack)
    }

    fn push_class_like_scope(&mut self) {
        access_modifier_predicates::push_class_like_scope(&mut self.macro_scope_stack);
    }

    fn push_non_macro_scope(&mut self) {
        access_modifier_predicates::push_def_scope(&mut self.macro_scope_stack);
    }

    fn push_wrapper_scope(&mut self) {
        access_modifier_predicates::push_wrapper_scope(&mut self.macro_scope_stack);
    }

    fn pop_scope(&mut self) {
        access_modifier_predicates::pop_scope(&mut self.macro_scope_stack);
    }

    fn is_target_block(&self, node: &ruby_prism::BlockNode<'_>) -> bool {
        let loc = node.location();
        loc.start_offset() == self.target_start && loc.end_offset() == self.target_end
    }

    fn visit_in_non_macro_scope<T>(&mut self, child: &T, visit: impl FnOnce(&mut Self, &T)) {
        self.push_non_macro_scope();
        visit(self, child);
        self.pop_scope();
    }
}

/// RuboCop's `class_constructor?` matches `Class.new`, `Module.new`,
/// `Struct.new` (with :new), and `Data.define`. These calls with blocks
/// act as class-like scopes in `in_macro_scope?`.
fn is_class_constructor_call(node: &ruby_prism::CallNode<'_>) -> bool {
    let method_name = node.name().as_slice();
    if let Some(receiver) = node.receiver() {
        if let Some(cr) = receiver.as_constant_read_node() {
            let const_name = cr.name().as_slice();
            return matches!(
                (const_name, method_name),
                (b"Class" | b"Module" | b"Struct", b"new") | (b"Data", b"define")
            );
        }
        // Handle ::Class.new (ConstantPathNode with no parent = cbase)
        if let Some(cp) = receiver.as_constant_path_node() {
            if cp.parent().is_none() {
                if let Some(name_node) = cp.name() {
                    let const_name = name_node.as_slice();
                    return matches!(
                        (const_name, method_name),
                        (b"Class" | b"Module" | b"Struct", b"new") | (b"Data", b"define")
                    );
                }
            }
        }
    }
    false
}

macro_rules! visit_node_as_non_macro_scope {
    ($method:ident, $node_ty:ty, $visit_fn:ident) => {
        fn $method(&mut self, node: &$node_ty) {
            if self.result.is_some() {
                return;
            }
            self.push_non_macro_scope();
            ruby_prism::$visit_fn(self, node);
            self.pop_scope();
        }
    };
}

impl<'pr> Visit<'pr> for MacroScopeChecker {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if self.result.is_some() {
            return;
        }

        if let Some(receiver) = node.receiver() {
            self.visit_in_non_macro_scope(&receiver, |this, receiver| this.visit(receiver));
            if self.result.is_some() {
                return;
            }
        }

        if let Some(arguments) = node.arguments() {
            self.visit_in_non_macro_scope(&arguments, |this, arguments| {
                this.visit_arguments_node(arguments)
            });
            if self.result.is_some() {
                return;
            }
        }

        if let Some(block_node) = node.block().and_then(|block| block.as_block_node()) {
            let block_is_class_like = is_class_constructor_call(node);

            if self.is_target_block(&block_node) {
                self.result = Some(block_is_class_like || self.current_macro_scope());
                return;
            }

            if block_is_class_like {
                self.push_class_like_scope();
            } else if self.current_macro_scope() {
                self.push_wrapper_scope();
            } else {
                self.push_non_macro_scope();
            }
            ruby_prism::visit_block_node(self, &block_node);
            self.pop_scope();
            return;
        }

        if let Some(block_argument) = node
            .block()
            .and_then(|block| block.as_block_argument_node())
        {
            self.visit_in_non_macro_scope(&block_argument, |this, block_argument| {
                this.visit_block_argument_node(block_argument)
            });
        }
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        self.push_non_macro_scope();
        ruby_prism::visit_def_node(self, node);
        self.pop_scope();
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        self.push_class_like_scope();
        ruby_prism::visit_class_node(self, node);
        self.pop_scope();
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        self.push_class_like_scope();
        ruby_prism::visit_module_node(self, node);
        self.pop_scope();
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        self.push_class_like_scope();
        ruby_prism::visit_singleton_class_node(self, node);
        self.pop_scope();
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        if node.rescue_clause().is_some() || node.ensure_clause().is_some() {
            self.push_non_macro_scope();
            ruby_prism::visit_begin_node(self, node);
            self.pop_scope();
            return;
        }

        ruby_prism::visit_begin_node(self, node);
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        self.push_non_macro_scope();
        ruby_prism::visit_lambda_node(self, node);
        self.pop_scope();
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        if self.result.is_some() {
            return;
        }

        self.visit_in_non_macro_scope(&node.predicate(), |this, predicate| this.visit(predicate));
        if self.result.is_some() {
            return;
        }

        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
            if self.result.is_some() {
                return;
            }
        }

        if let Some(subsequent) = node.subsequent() {
            self.visit(&subsequent);
        }
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        if self.result.is_some() {
            return;
        }

        self.visit_in_non_macro_scope(&node.predicate(), |this, predicate| this.visit(predicate));
        if self.result.is_some() {
            return;
        }

        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
            if self.result.is_some() {
                return;
            }
        }

        if let Some(else_clause) = node.else_clause() {
            self.visit_else_node(&else_clause);
        }
    }

    visit_node_as_non_macro_scope!(visit_and_node, ruby_prism::AndNode<'pr>, visit_and_node);
    visit_node_as_non_macro_scope!(visit_or_node, ruby_prism::OrNode<'pr>, visit_or_node);
    visit_node_as_non_macro_scope!(
        visit_rescue_node,
        ruby_prism::RescueNode<'pr>,
        visit_rescue_node
    );
    visit_node_as_non_macro_scope!(
        visit_ensure_node,
        ruby_prism::EnsureNode<'pr>,
        visit_ensure_node
    );
    visit_node_as_non_macro_scope!(visit_case_node, ruby_prism::CaseNode<'pr>, visit_case_node);
    visit_node_as_non_macro_scope!(
        visit_case_match_node,
        ruby_prism::CaseMatchNode<'pr>,
        visit_case_match_node
    );
    visit_node_as_non_macro_scope!(
        visit_call_and_write_node,
        ruby_prism::CallAndWriteNode<'pr>,
        visit_call_and_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_call_operator_write_node,
        ruby_prism::CallOperatorWriteNode<'pr>,
        visit_call_operator_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_call_or_write_node,
        ruby_prism::CallOrWriteNode<'pr>,
        visit_call_or_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_class_variable_and_write_node,
        ruby_prism::ClassVariableAndWriteNode<'pr>,
        visit_class_variable_and_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_class_variable_operator_write_node,
        ruby_prism::ClassVariableOperatorWriteNode<'pr>,
        visit_class_variable_operator_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_class_variable_or_write_node,
        ruby_prism::ClassVariableOrWriteNode<'pr>,
        visit_class_variable_or_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_class_variable_write_node,
        ruby_prism::ClassVariableWriteNode<'pr>,
        visit_class_variable_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_constant_and_write_node,
        ruby_prism::ConstantAndWriteNode<'pr>,
        visit_constant_and_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_constant_operator_write_node,
        ruby_prism::ConstantOperatorWriteNode<'pr>,
        visit_constant_operator_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_constant_or_write_node,
        ruby_prism::ConstantOrWriteNode<'pr>,
        visit_constant_or_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_constant_path_and_write_node,
        ruby_prism::ConstantPathAndWriteNode<'pr>,
        visit_constant_path_and_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_constant_path_operator_write_node,
        ruby_prism::ConstantPathOperatorWriteNode<'pr>,
        visit_constant_path_operator_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_constant_path_or_write_node,
        ruby_prism::ConstantPathOrWriteNode<'pr>,
        visit_constant_path_or_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_constant_path_write_node,
        ruby_prism::ConstantPathWriteNode<'pr>,
        visit_constant_path_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_constant_write_node,
        ruby_prism::ConstantWriteNode<'pr>,
        visit_constant_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_global_variable_and_write_node,
        ruby_prism::GlobalVariableAndWriteNode<'pr>,
        visit_global_variable_and_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_global_variable_operator_write_node,
        ruby_prism::GlobalVariableOperatorWriteNode<'pr>,
        visit_global_variable_operator_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_global_variable_or_write_node,
        ruby_prism::GlobalVariableOrWriteNode<'pr>,
        visit_global_variable_or_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_global_variable_write_node,
        ruby_prism::GlobalVariableWriteNode<'pr>,
        visit_global_variable_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_index_and_write_node,
        ruby_prism::IndexAndWriteNode<'pr>,
        visit_index_and_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_index_operator_write_node,
        ruby_prism::IndexOperatorWriteNode<'pr>,
        visit_index_operator_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_index_or_write_node,
        ruby_prism::IndexOrWriteNode<'pr>,
        visit_index_or_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_instance_variable_and_write_node,
        ruby_prism::InstanceVariableAndWriteNode<'pr>,
        visit_instance_variable_and_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_instance_variable_operator_write_node,
        ruby_prism::InstanceVariableOperatorWriteNode<'pr>,
        visit_instance_variable_operator_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_instance_variable_or_write_node,
        ruby_prism::InstanceVariableOrWriteNode<'pr>,
        visit_instance_variable_or_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_instance_variable_write_node,
        ruby_prism::InstanceVariableWriteNode<'pr>,
        visit_instance_variable_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_local_variable_and_write_node,
        ruby_prism::LocalVariableAndWriteNode<'pr>,
        visit_local_variable_and_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_local_variable_operator_write_node,
        ruby_prism::LocalVariableOperatorWriteNode<'pr>,
        visit_local_variable_operator_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_local_variable_or_write_node,
        ruby_prism::LocalVariableOrWriteNode<'pr>,
        visit_local_variable_or_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_local_variable_write_node,
        ruby_prism::LocalVariableWriteNode<'pr>,
        visit_local_variable_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_match_write_node,
        ruby_prism::MatchWriteNode<'pr>,
        visit_match_write_node
    );
    visit_node_as_non_macro_scope!(
        visit_multi_write_node,
        ruby_prism::MultiWriteNode<'pr>,
        visit_multi_write_node
    );
}

fn is_block_in_macro_scope(
    parse_result: &ruby_prism::ParseResult<'_>,
    block_start: usize,
    block_end: usize,
) -> bool {
    let mut checker = MacroScopeChecker::new(block_start, block_end);
    checker.visit(&parse_result.node());
    checker.result.unwrap_or(true)
}

impl Cop for AccessModifierIndentation {
    fn name(&self) -> &'static str {
        "Layout/AccessModifierIndentation"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            BLOCK_NODE,
            CALL_NODE,
            CLASS_NODE,
            MODULE_NODE,
            SINGLETON_CLASS_NODE,
        ]
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
        let style = config.get_str("EnforcedStyle", "indent");
        let indent_width = config.get_usize("IndentationWidth", 2);

        // We need a class, module, sclass, or block node that contains access modifiers.
        // Extract the body and the offset of the `end` keyword (used to compute expected
        // indentation, matching RuboCop's `column_offset_between(modifier, end_range)`).
        let (body, end_offset, container_start_offset) =
            if let Some(class_node) = node.as_class_node() {
                match class_node.body() {
                    Some(b) => (
                        b,
                        class_node.end_keyword_loc().start_offset(),
                        class_node.location().start_offset(),
                    ),
                    None => return,
                }
            } else if let Some(module_node) = node.as_module_node() {
                match module_node.body() {
                    Some(b) => (
                        b,
                        module_node.end_keyword_loc().start_offset(),
                        module_node.location().start_offset(),
                    ),
                    None => return,
                }
            } else if let Some(sclass_node) = node.as_singleton_class_node() {
                match sclass_node.body() {
                    Some(b) => (
                        b,
                        sclass_node.end_keyword_loc().start_offset(),
                        sclass_node.location().start_offset(),
                    ),
                    None => return,
                }
            } else if let Some(block_node) = node.as_block_node() {
                // RuboCop's bare_access_modifier? checks in_macro_scope?, which
                // returns false for blocks inside def/defs. Skip blocks that are
                // not in macro scope (e.g., class_eval do...end inside a method).
                let loc = block_node.location();
                let block_start = loc.start_offset();
                let block_end = loc.end_offset();
                if !is_block_in_macro_scope(_parse_result, block_start, block_end) {
                    return;
                }
                match block_node.body() {
                    Some(b) => (
                        b,
                        block_node.closing_loc().start_offset(),
                        block_node.location().start_offset(),
                    ),
                    None => return,
                }
            } else {
                return;
            };

        // RuboCop measures the column offset between the access modifier and the
        // `end` keyword of the enclosing scope.  For `indent` style the modifier
        // should be one `IndentationWidth` to the right of `end`; for `outdent`
        // it should be at the same column as `end`.
        let (end_line, end_col) = source.offset_to_line_col(end_offset);
        let (container_line, _) = source.offset_to_line_col(container_start_offset);

        let stmts = body_statements(body);

        // RuboCop only checks when the body is `begin_type?` (2+ statements).
        // A body with only an access modifier and no other statements is not flagged.
        if stmts.len() < 2 {
            return;
        }

        for stmt in stmts {
            let call = match stmt.as_call_node() {
                Some(c) => c,
                None => continue,
            };

            if !access_modifier_predicates::is_bare_access_modifier(&call) {
                continue;
            }

            let (mod_line, mod_col) = source.offset_to_line_col(call.location().start_offset());

            // Same line as container keyword — skip
            if mod_line == container_line {
                continue;
            }

            // Same line as end keyword — skip
            if mod_line == end_line {
                continue;
            }

            let expected_col = match style {
                "outdent" => end_col,
                _ => end_col + indent_width,
            };

            if mod_col != expected_col {
                let style_word = if style == "outdent" {
                    "Outdent"
                } else {
                    "Indent"
                };
                let modifier_name =
                    std::str::from_utf8(call.name().as_slice()).unwrap_or("private");
                diagnostics.push(self.diagnostic(
                    source,
                    mod_line,
                    mod_col,
                    format!("{style_word} access modifiers like `{modifier_name}`."),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full_with_config;
    use std::collections::HashMap;

    crate::cop_fixture_tests!(
        AccessModifierIndentation,
        "cops/layout/access_modifier_indentation"
    );

    #[test]
    fn honors_indentation_width_for_block_bodies() {
        let config = CopConfig {
            options: HashMap::from([(
                "IndentationWidth".into(),
                serde_yml::Value::Number(4.into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"describe Foo do\n    private\n    def helper; end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierIndentation, source, config);
        assert!(
            diags.is_empty(),
            "width 4 should accept a 4-space access modifier inside a block: {:?}",
            diags
        );
    }

    #[test]
    fn outdent_skips_block_inside_def() {
        // RuboCop's bare_access_modifier? returns false for `private` inside
        // a block that's inside a method definition (not in macro scope).
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("outdent".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"class Foo\n  def self.setup(base)\n    base.class_eval do\n      def color; end\n      private\n      def secret; end\n    end\n  end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierIndentation, source, config);
        assert!(
            diags.is_empty(),
            "block inside def is not in macro scope, should not flag: {:?}",
            diags
        );
    }

    #[test]
    fn outdent_still_checks_block_in_class_body() {
        // A block directly in a class body IS in macro scope.
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("outdent".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"class Foo\n  class_eval do\n    def color; end\n    private\n    def secret; end\n  end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierIndentation, source, config);
        assert_eq!(
            diags.len(),
            1,
            "block in class body is in macro scope, should flag: {:?}",
            diags
        );
    }

    #[test]
    fn outdent_skips_block_inside_standalone_def() {
        // A block inside a standalone def (no enclosing class) is not in macro scope.
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("outdent".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def setup(base)\n  base.class_eval do\n    def color; end\n    private\n    def secret; end\n  end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierIndentation, source, config);
        assert!(
            diags.is_empty(),
            "block inside standalone def is not in macro scope: {:?}",
            diags
        );
    }

    #[test]
    fn indent_skips_block_inside_def() {
        // Same check for indent style — blocks inside defs are not in macro scope.
        let source = b"class Foo\n  def self.setup(base)\n    base.class_eval do\n      def color; end\n      private\n      def secret; end\n    end\n  end\nend\n";
        let diags =
            run_cop_full_with_config(&AccessModifierIndentation, source, CopConfig::default());
        assert!(
            diags.is_empty(),
            "block inside def is not in macro scope (indent style): {:?}",
            diags
        );
    }

    #[test]
    fn outdent_checks_class_new_block_inside_def() {
        // Class.new do...end is a class_constructor? in RuboCop — it resets
        // macro scope even inside a def. The cop SHOULD check access modifiers here.
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("outdent".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"class Foo\n  def setup\n    @klass = Class.new do\n      private\n      def secret; end\n    end\n  end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierIndentation, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Class.new block is a class constructor (macro scope), should flag: {:?}",
            diags
        );
    }

    #[test]
    fn outdent_checks_module_new_block_inside_def() {
        // Module.new do...end is also a class_constructor? — resets macro scope.
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("outdent".into()),
            )]),
            ..CopConfig::default()
        };
        let source =
            b"def setup\n  @mod = Module.new do\n    private\n    def secret; end\n  end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierIndentation, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Module.new block is a class constructor (macro scope), should flag: {:?}",
            diags
        );
    }

    #[test]
    fn outdent_no_offense_fixture() {
        // Non-class-constructor block inside constant assignment (casgn) is not
        // in macro scope — RuboCop's bare_access_modifier? returns false.
        let fixture = include_bytes!(
            "../../../tests/fixtures/cops/layout/access_modifier_indentation/no_offense.outdent.rb"
        );
        let fixture_str = std::str::from_utf8(fixture).expect("fixture must be valid UTF-8");
        let source = fixture_str
            .strip_prefix("# nitrocop-config: EnforcedStyle: outdent\n")
            .expect("fixture should start with outdent config directive");
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("outdent".into()),
            )]),
            ..CopConfig::default()
        };
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &AccessModifierIndentation,
            source.as_bytes(),
            config,
        );
    }

    #[test]
    fn outdent_skips_non_class_constructor_block_in_casgn() {
        // CommandClass.new is NOT a class_constructor (not Class/Module/Struct/Data).
        // The constant assignment (casgn) breaks the macro scope chain,
        // so `private` is not a bare access modifier and should not be checked.
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("outdent".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"module Foo\n  Bar = SomeBuilder.new do\n    def call; end\n    private\n    def secret; end\n  end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierIndentation, source, config);
        assert!(
            diags.is_empty(),
            "non-class-constructor block in casgn is not in macro scope: {:?}",
            diags
        );
    }

    #[test]
    fn outdent_still_checks_class_constructor_block_in_casgn() {
        // Class.new IS a class_constructor — it resets macro scope even inside casgn.
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("outdent".into()),
            )]),
            ..CopConfig::default()
        };
        let source =
            b"module Foo\n  Bar = Class.new do\n    private\n    def secret; end\n  end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierIndentation, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Class.new block in casgn is a class constructor (macro scope), should flag: {:?}",
            diags
        );
    }

    #[test]
    fn flags_under_indented_block_bodies_when_indentation_width_is_four() {
        let config = CopConfig {
            options: HashMap::from([(
                "IndentationWidth".into(),
                serde_yml::Value::Number(4.into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"describe Foo do\n  private\n    def helper; end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierIndentation, source, config);
        assert_eq!(diags.len(), 1, "expected one offense, got: {:?}", diags);
        assert_eq!(diags[0].message, "Indent access modifiers like `private`.");
    }
}
