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
    in_def: bool,
    result: Option<bool>,
}

impl MacroScopeChecker {
    fn new(target_start: usize, target_end: usize) -> Self {
        Self {
            target_start,
            target_end,
            in_def: false,
            result: None,
        }
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

impl<'pr> Visit<'pr> for MacroScopeChecker {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        // Class.new/Module.new/Struct.new/Data.define blocks act as class-like
        // scopes (class_constructor? in RuboCop's in_macro_scope?), resetting
        // the macro scope just like class/module/sclass nodes.
        if is_class_constructor_call(node) {
            let prev = self.in_def;
            self.in_def = false;
            ruby_prism::visit_call_node(self, node);
            self.in_def = prev;
        } else {
            ruby_prism::visit_call_node(self, node);
        }
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        let prev = self.in_def;
        self.in_def = true;
        ruby_prism::visit_def_node(self, node);
        self.in_def = prev;
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        let prev = self.in_def;
        self.in_def = false;
        ruby_prism::visit_class_node(self, node);
        self.in_def = prev;
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        let prev = self.in_def;
        self.in_def = false;
        ruby_prism::visit_module_node(self, node);
        self.in_def = prev;
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        let prev = self.in_def;
        self.in_def = false;
        ruby_prism::visit_singleton_class_node(self, node);
        self.in_def = prev;
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        if self.result.is_some() {
            return;
        }
        let loc = node.location();
        let start = loc.start_offset();
        let end = loc.end_offset();
        if start == self.target_start && end == self.target_end {
            self.result = Some(!self.in_def);
            return;
        }
        ruby_prism::visit_block_node(self, node);
    }
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
