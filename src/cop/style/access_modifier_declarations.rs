use crate::cop::shared::access_modifier_predicates;
use crate::cop::shared::node_type_groups;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Checks that access modifiers are declared in the correct style (group or inline).
///
/// ## Investigation (2026-04-04)
///
/// Three upstream-compatibility gaps remained after the earlier block-parent fix:
///
/// 1. We skipped every `StatementsNode` nested anywhere inside a `def`, which hid real
///    offenses in nested class-like scopes such as `Class.new do ... end` and
///    `class << obj` inside methods.
/// 2. The proc/lambda owner override leaked past the direct `Proc.new { ... }` block into
///    nested DSL blocks like `operation :foo do ... end`, suppressing inline modifier
///    offenses that RuboCop still reports there.
/// 3. `case` / `when` bodies still counted as macro wrappers, but RuboCop treats them as
///    scope breaks, so direct `private def ...` inside a `when` branch must be ignored.
///
/// Fix: keep a small group-style scope flag that wrappers inherit, reset it for
/// ordinary method bodies, rescuing `begin`, and `case` branches, and explicitly
/// re-enable it for nested class/module/sclass and class-constructor blocks.
/// `Proc.new` / `lambda` remain a direct-parent exemption only: they suppress access
/// modifiers in their own bodies, but nested DSL/class-like blocks underneath them
/// still get checked once the proc-block override is consumed.
///
/// ## Inline style variant fix (2026-04-05)
///
/// The `EnforcedStyle: inline` variant had 1,238 FPs because the inline-style check
/// in `visit_call_node` flagged every bare access modifier in macro scope. RuboCop only
/// flags bare modifiers when there are grouped `def` nodes following them (up to the
/// next bare modifier), via `select_grouped_def_nodes(node).any?`. A bare `private`
/// followed only by `attr_reader` or other non-def calls is not an offense.
///
/// Fix: moved inline-style detection from `visit_call_node` to a new
/// `check_inline_style_statements` method called from `visit_statements_node`, where
/// we have access to sibling nodes. For each bare modifier, we look at right siblings
/// (stopping at the next bare modifier) and only flag if any sibling is a `DefNode`.
pub struct AccessModifierDeclarations;

// Uses access_modifier_predicates for access modifier detection.

#[derive(Clone, Copy, Eq, PartialEq)]
enum StatementsOwnerKind {
    Other,
    Root,
    Block,
    Def,
    If,
    CaseLike,
    ProcLikeBlock,
    RescuingBegin,
}

struct AccessModifierVisitor<'a> {
    source: &'a SourceFile,
    cop: &'a AccessModifierDeclarations,
    enforced_style: &'a str,
    allow_modifiers_on_symbols: bool,
    allow_modifiers_on_attrs: bool,
    allow_modifiers_on_alias_method: bool,
    diagnostics: Vec<Diagnostic>,
    /// Macro scope stack for access modifier detection.
    macro_scope_stack: Vec<access_modifier_predicates::MacroScope>,
    /// Whether direct access modifiers in the current wrapper chain are checkable in
    /// group style. Ordinary `def` bodies, rescuing `begin`, and `case`/`when`
    /// wrappers disable this until a nested class-like scope resets it.
    group_scope_active: bool,
    /// Synthetic owner kind for the next statements node we visit.
    statements_owner_kind: StatementsOwnerKind,
    /// Optional owner override for the direct block child of the current call node.
    next_block_owner_kind: Option<StatementsOwnerKind>,
    /// Optional group-scope override for the direct block child of the current call node.
    next_block_group_scope: Option<bool>,
}

struct ModifierClassification<'a> {
    method_name: &'a str,
    is_inlined: bool,
    is_symbol_pattern: bool,
}

/// Classify an access modifier call. Returns metadata for non-allowed access
/// modifier sends, or None when the call should be skipped entirely.
fn classify_access_modifier<'pr>(
    call: &ruby_prism::CallNode<'pr>,
    allow_modifiers_on_symbols: bool,
    allow_modifiers_on_attrs: bool,
    allow_modifiers_on_alias_method: bool,
) -> Option<ModifierClassification<'pr>> {
    if !access_modifier_predicates::is_access_modifier_declaration(call) {
        return None;
    }
    let method_name = std::str::from_utf8(call.name().as_slice()).unwrap_or("");

    let args = match call.arguments() {
        Some(arguments) => arguments,
        None => {
            return Some(ModifierClassification {
                method_name,
                is_inlined: false,
                is_symbol_pattern: false,
            });
        }
    };

    let arg_list: Vec<_> = args.arguments().iter().collect();
    if arg_list.is_empty() {
        return Some(ModifierClassification {
            method_name,
            is_inlined: false,
            is_symbol_pattern: false,
        });
    }

    let is_symbol_pattern = access_modifier_with_symbol(&arg_list);
    if is_symbol_pattern && allow_modifiers_on_symbols {
        return None;
    }

    let first_arg = &arg_list[0];
    if allow_modifiers_on_attrs {
        if let Some(inner_call) = first_arg.as_call_node() {
            let inner_name = std::str::from_utf8(inner_call.name().as_slice()).unwrap_or("");
            if matches!(
                inner_name,
                "attr_reader" | "attr_writer" | "attr_accessor" | "attr"
            ) {
                return None;
            }
        }
    }

    if allow_modifiers_on_alias_method {
        if let Some(inner_call) = first_arg.as_call_node() {
            let inner_name = std::str::from_utf8(inner_call.name().as_slice()).unwrap_or("");
            if inner_name == "alias_method" {
                return None;
            }
        }
    }

    Some(ModifierClassification {
        method_name,
        is_inlined: true,
        is_symbol_pattern,
    })
}

fn access_modifier_with_symbol(args: &[ruby_prism::Node<'_>]) -> bool {
    !args.is_empty()
        && (args.iter().all(|arg| arg.as_symbol_node().is_some())
            || (args.len() == 1 && symbol_splat_arg(&args[0])))
}

fn symbol_splat_arg(arg: &ruby_prism::Node<'_>) -> bool {
    let Some(splat) = arg.as_splat_node() else {
        return false;
    };

    let Some(expression) = splat.expression() else {
        return false;
    };

    expression
        .as_array_node()
        .is_some_and(|array| is_percent_symbol_array(&array))
        || expression.as_constant_read_node().is_some()
        || expression.as_constant_path_node().is_some()
        || expression.as_call_node().is_some_and(|call| {
            call.block()
                .is_none_or(|block| !node_type_groups::is_any_block_node(&block))
        })
}

fn is_percent_symbol_array(array: &ruby_prism::ArrayNode<'_>) -> bool {
    let Some(opening_loc) = array.opening_loc() else {
        return false;
    };

    let opening = opening_loc.as_slice();
    opening.starts_with(b"%i") || opening.starts_with(b"%I")
}

fn call_is_proc_like(call: &ruby_prism::CallNode<'_>) -> bool {
    let method_name = std::str::from_utf8(call.name().as_slice()).unwrap_or("");
    if call.receiver().is_none() {
        return matches!(method_name, "proc" | "lambda");
    }

    if method_name != "new" {
        return false;
    }

    let Some(receiver) = call.receiver() else {
        return false;
    };

    let slice = receiver.location().as_slice();
    slice == b"Proc" || slice == b"::Proc" || slice.ends_with(b"::Proc")
}

fn call_is_class_constructor(call: &ruby_prism::CallNode<'_>) -> bool {
    let method_name = std::str::from_utf8(call.name().as_slice()).unwrap_or("");
    let Some(receiver) = call.receiver() else {
        return false;
    };

    let receiver_source = receiver.location().as_slice();
    match method_name {
        "new" => matches!(
            receiver_source,
            b"Class" | b"::Class" | b"Module" | b"::Module" | b"Struct" | b"::Struct"
        ),
        "define" => matches!(receiver_source, b"Data" | b"::Data"),
        _ => false,
    }
}

fn has_corresponding_def_nodes<'pr>(
    classification: &ModifierClassification<'pr>,
    args: &[ruby_prism::Node<'pr>],
    stmts: &[ruby_prism::Node<'pr>],
) -> bool {
    if !classification.is_symbol_pattern {
        return true;
    }

    let method_names: Vec<Vec<u8>> = args
        .iter()
        .filter_map(|arg| arg.as_symbol_node())
        .map(|sym| sym.unescaped().to_vec())
        .collect();

    if method_names.is_empty() {
        return false;
    }

    let defined_names: Vec<Vec<u8>> = stmts
        .iter()
        .filter_map(|stmt| stmt.as_def_node())
        .map(|def| def.name_loc().as_slice().to_vec())
        .collect();

    method_names
        .iter()
        .all(|method_name| defined_names.contains(method_name))
}

/// Info about an access modifier at a given position in a body's statement list.
struct ModifierInfo<'a> {
    method_name: &'a str,
    is_inlined: bool,
    has_corresponding_def_nodes: bool,
    start_offset: usize,
}

impl AccessModifierVisitor<'_> {
    fn check_inline_style_statements<'pr>(&mut self, stmts: &[ruby_prism::Node<'pr>]) {
        if self.enforced_style != "inline" {
            return;
        }

        if !access_modifier_predicates::in_macro_scope(&self.macro_scope_stack) {
            return;
        }

        for (index, stmt) in stmts.iter().enumerate() {
            let Some(call) = stmt.as_call_node() else {
                continue;
            };

            if !access_modifier_predicates::is_bare_access_modifier(&call) {
                continue;
            }

            // RuboCop inline style: only flag if there are grouped def nodes
            // following this bare modifier (up to the next bare access modifier).
            // This mirrors RuboCop's `select_grouped_def_nodes(node).any?`.
            let has_grouped_defs = stmts[index + 1..]
                .iter()
                .take_while(|sibling| {
                    !sibling
                        .as_call_node()
                        .is_some_and(|c| access_modifier_predicates::is_bare_access_modifier(&c))
                })
                .any(|sibling| sibling.as_def_node().is_some());

            if !has_grouped_defs {
                continue;
            }

            let loc = call.location();
            let (line, column) = self.source.offset_to_line_col(loc.start_offset());
            self.diagnostics.push(self.cop.diagnostic(
                self.source,
                line,
                column,
                format!(
                    "`{}` should be inlined in method definitions.",
                    std::str::from_utf8(call.name().as_slice()).unwrap_or("private")
                ),
            ));
        }
    }

    fn check_group_style_statements<'pr>(&mut self, stmts: &[ruby_prism::Node<'pr>]) {
        if self.enforced_style != "group" || !self.group_scope_active {
            return;
        }

        let direct_parent_is_block =
            matches!(self.statements_owner_kind, StatementsOwnerKind::Block) && stmts.len() == 1;
        let direct_parent_is_if =
            matches!(self.statements_owner_kind, StatementsOwnerKind::If) && stmts.len() == 1;
        let direct_parent_is_proc_like_block = matches!(
            self.statements_owner_kind,
            StatementsOwnerKind::ProcLikeBlock
        );
        let direct_parent_is_rescuing_begin = matches!(
            self.statements_owner_kind,
            StatementsOwnerKind::RescuingBegin
        );
        let root_statements = matches!(self.statements_owner_kind, StatementsOwnerKind::Root);

        let infos: Vec<Option<ModifierInfo>> = stmts
            .iter()
            .map(|stmt| {
                let call = stmt.as_call_node()?;
                let classification = classify_access_modifier(
                    &call,
                    self.allow_modifiers_on_symbols,
                    self.allow_modifiers_on_attrs,
                    self.allow_modifiers_on_alias_method,
                )?;

                if direct_parent_is_block
                    || direct_parent_is_if
                    || direct_parent_is_proc_like_block
                    || direct_parent_is_rescuing_begin
                {
                    return None;
                }

                if root_statements && classification.is_symbol_pattern {
                    return None;
                }

                let args = call.arguments()?;
                let arg_list: Vec<_> = args.arguments().iter().collect();

                Some(ModifierInfo {
                    method_name: classification.method_name,
                    is_inlined: classification.is_inlined,
                    has_corresponding_def_nodes: has_corresponding_def_nodes(
                        &classification,
                        &arg_list,
                        stmts,
                    ),
                    start_offset: call.location().start_offset(),
                })
            })
            .collect();

        for (index, info) in infos.iter().enumerate() {
            let Some(info) = info else {
                continue;
            };

            if !info.is_inlined {
                continue;
            }

            let has_right_sibling_same_inline_modifier = infos[index + 1..].iter().any(|other| {
                matches!(
                    other,
                    Some(other_info)
                        if other_info.is_inlined
                            && other_info.has_corresponding_def_nodes
                            && other_info.method_name == info.method_name
                )
            });

            if has_right_sibling_same_inline_modifier {
                continue;
            }

            let (line, column) = self.source.offset_to_line_col(info.start_offset);
            self.diagnostics.push(self.cop.diagnostic(
                self.source,
                line,
                column,
                format!(
                    "`{}` should not be inlined in method definitions.",
                    info.method_name
                ),
            ));
        }
    }
}

impl<'pr> Visit<'pr> for AccessModifierVisitor<'_> {
    fn visit_program_node(&mut self, node: &ruby_prism::ProgramNode<'pr>) {
        let saved = self.statements_owner_kind;
        let saved_group_scope = self.group_scope_active;
        self.group_scope_active = true;
        self.statements_owner_kind = StatementsOwnerKind::Root;
        ruby_prism::visit_program_node(self, node);
        self.statements_owner_kind = saved;
        self.group_scope_active = saved_group_scope;
    }

    fn visit_statements_node(&mut self, node: &ruby_prism::StatementsNode<'pr>) {
        let stmts: Vec<_> = node.body().iter().collect();
        self.check_group_style_statements(&stmts);
        self.check_inline_style_statements(&stmts);

        let saved = self.statements_owner_kind;
        self.statements_owner_kind = StatementsOwnerKind::Other;
        ruby_prism::visit_statements_node(self, node);
        self.statements_owner_kind = saved;
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        access_modifier_predicates::push_class_like_scope(&mut self.macro_scope_stack);
        let saved_owner = self.statements_owner_kind;
        let saved_group_scope = self.group_scope_active;
        self.group_scope_active = true;
        self.statements_owner_kind = StatementsOwnerKind::Other;
        ruby_prism::visit_class_node(self, node);
        self.statements_owner_kind = saved_owner;
        self.group_scope_active = saved_group_scope;
        access_modifier_predicates::pop_scope(&mut self.macro_scope_stack);
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        access_modifier_predicates::push_class_like_scope(&mut self.macro_scope_stack);
        let saved_owner = self.statements_owner_kind;
        let saved_group_scope = self.group_scope_active;
        self.group_scope_active = true;
        self.statements_owner_kind = StatementsOwnerKind::Other;
        ruby_prism::visit_module_node(self, node);
        self.statements_owner_kind = saved_owner;
        self.group_scope_active = saved_group_scope;
        access_modifier_predicates::pop_scope(&mut self.macro_scope_stack);
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        access_modifier_predicates::push_class_like_scope(&mut self.macro_scope_stack);
        let saved_owner = self.statements_owner_kind;
        let saved_group_scope = self.group_scope_active;
        self.group_scope_active = true;
        self.statements_owner_kind = StatementsOwnerKind::Other;
        ruby_prism::visit_singleton_class_node(self, node);
        self.statements_owner_kind = saved_owner;
        self.group_scope_active = saved_group_scope;
        access_modifier_predicates::pop_scope(&mut self.macro_scope_stack);
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        access_modifier_predicates::push_wrapper_scope(&mut self.macro_scope_stack);
        let saved_owner = self.statements_owner_kind;
        let saved_group_scope = self.group_scope_active;
        self.statements_owner_kind = self
            .next_block_owner_kind
            .take()
            .unwrap_or(StatementsOwnerKind::Block);
        self.group_scope_active = self
            .next_block_group_scope
            .take()
            .unwrap_or(self.group_scope_active);
        ruby_prism::visit_block_node(self, node);
        self.statements_owner_kind = saved_owner;
        self.group_scope_active = saved_group_scope;
        access_modifier_predicates::pop_scope(&mut self.macro_scope_stack);
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        access_modifier_predicates::push_def_scope(&mut self.macro_scope_stack);
        let saved_owner = self.statements_owner_kind;
        self.statements_owner_kind = StatementsOwnerKind::ProcLikeBlock;
        ruby_prism::visit_lambda_node(self, node);
        self.statements_owner_kind = saved_owner;
        access_modifier_predicates::pop_scope(&mut self.macro_scope_stack);
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        let saved = self.statements_owner_kind;
        self.statements_owner_kind = StatementsOwnerKind::If;
        ruby_prism::visit_if_node(self, node);
        self.statements_owner_kind = saved;
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        let saved = self.statements_owner_kind;
        self.statements_owner_kind = StatementsOwnerKind::If;
        ruby_prism::visit_unless_node(self, node);
        self.statements_owner_kind = saved;
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        let saved = self.statements_owner_kind;
        let saved_group_scope = self.group_scope_active;
        let is_pure_begin = node.rescue_clause().is_none()
            && node.ensure_clause().is_none()
            && node.else_clause().is_none();
        if !is_pure_begin {
            self.statements_owner_kind = StatementsOwnerKind::RescuingBegin;
            self.group_scope_active = false;
        }
        ruby_prism::visit_begin_node(self, node);
        self.statements_owner_kind = saved;
        self.group_scope_active = saved_group_scope;
    }

    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'pr>) {
        let saved = self.statements_owner_kind;
        let saved_group_scope = self.group_scope_active;
        self.group_scope_active = false;
        self.statements_owner_kind = StatementsOwnerKind::CaseLike;
        ruby_prism::visit_case_node(self, node);
        self.statements_owner_kind = saved;
        self.group_scope_active = saved_group_scope;
    }

    fn visit_case_match_node(&mut self, node: &ruby_prism::CaseMatchNode<'pr>) {
        let saved = self.statements_owner_kind;
        let saved_group_scope = self.group_scope_active;
        self.group_scope_active = false;
        self.statements_owner_kind = StatementsOwnerKind::CaseLike;
        ruby_prism::visit_case_match_node(self, node);
        self.statements_owner_kind = saved;
        self.group_scope_active = saved_group_scope;
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let saved = self.statements_owner_kind;
        let saved_group_scope = self.group_scope_active;
        self.group_scope_active = false;
        self.statements_owner_kind = StatementsOwnerKind::Def;
        ruby_prism::visit_def_node(self, node);
        self.statements_owner_kind = saved;
        self.group_scope_active = saved_group_scope;
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let saved_next_block_owner_kind = self.next_block_owner_kind;
        let saved_next_block_group_scope = self.next_block_group_scope;
        if node
            .block()
            .and_then(|block| block.as_block_node())
            .is_some()
        {
            if call_is_proc_like(node) {
                self.next_block_owner_kind = Some(StatementsOwnerKind::ProcLikeBlock);
            } else if call_is_class_constructor(node) {
                self.next_block_group_scope = Some(true);
            }
        }

        // Inline-style checks are now handled in check_inline_style_statements
        // (visit_statements_node) where we have access to sibling nodes to check
        // for following def nodes.
        ruby_prism::visit_call_node(self, node);
        self.next_block_owner_kind = saved_next_block_owner_kind;
        self.next_block_group_scope = saved_next_block_group_scope;
    }
}

impl Cop for AccessModifierDeclarations {
    fn name(&self) -> &'static str {
        "Style/AccessModifierDeclarations"
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
        let enforced_style = config.get_str("EnforcedStyle", "group");
        let allow_modifiers_on_symbols = config.get_bool("AllowModifiersOnSymbols", true);
        let allow_modifiers_on_attrs = config.get_bool("AllowModifiersOnAttrs", true);
        let allow_modifiers_on_alias_method = config.get_bool("AllowModifiersOnAliasMethod", true);

        let mut visitor = AccessModifierVisitor {
            source,
            cop: self,
            enforced_style,
            allow_modifiers_on_symbols,
            allow_modifiers_on_attrs,
            allow_modifiers_on_alias_method,
            diagnostics: Vec::new(),
            macro_scope_stack: vec![],
            group_scope_active: true,
            statements_owner_kind: StatementsOwnerKind::Other,
            next_block_owner_kind: None,
            next_block_group_scope: None,
        };

        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::CopConfig;
    use crate::testutil::run_cop_full_with_config;

    crate::cop_fixture_tests!(
        AccessModifierDeclarations,
        "cops/style/access_modifier_declarations"
    );

    fn inline_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("inline".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn inline_style_flags_bare_modifier_with_following_defs() {
        // Bare `private` followed by def nodes → offense
        let source = b"class Foo\n  private\n\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierDeclarations, source, inline_config());
        assert_eq!(
            diags.len(),
            1,
            "inline style should flag bare modifier with following defs"
        );
    }

    #[test]
    fn inline_style_no_offense_bare_modifier_without_defs() {
        // Bare `private` followed by only attr_reader → no offense
        let source = b"class Foo\n  private\n\n  attr_reader :bar\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierDeclarations, source, inline_config());
        assert!(
            diags.is_empty(),
            "inline style should NOT flag bare modifier without following defs, got: {:?}",
            diags
        );
    }

    #[test]
    fn inline_style_no_offense_bare_modifier_alone() {
        // Bare `private` with nothing after it → no offense
        let source = b"class Foo\n  private\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierDeclarations, source, inline_config());
        assert!(
            diags.is_empty(),
            "inline style should NOT flag bare modifier with no following statements"
        );
    }

    #[test]
    fn inline_style_flags_bare_modifier_with_mixed_content() {
        // Bare `private` followed by attr_reader AND def → offense (def is present)
        let source = b"class Mixed\n  private\n\n  attr_reader :something\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierDeclarations, source, inline_config());
        assert_eq!(
            diags.len(),
            1,
            "inline style should flag bare modifier when defs are among siblings"
        );
    }

    #[test]
    fn inline_style_flags_module_function() {
        let source = b"class Foo\n  module_function\n\n  def helper; end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierDeclarations, source, inline_config());
        assert_eq!(
            diags.len(),
            1,
            "inline style should flag bare module_function with following defs"
        );
    }

    #[test]
    fn inline_style_no_offense_for_inline_modifiers() {
        // Inline `private def` → no offense in inline style
        let source = b"class Foo\n  private def bar; end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierDeclarations, source, inline_config());
        assert!(
            diags.is_empty(),
            "inline style should NOT flag already-inlined modifiers"
        );
    }

    #[test]
    fn inline_style_stops_at_next_bare_modifier() {
        // `private` followed by `protected` then def → private has no defs before protected
        let source = b"class Foo\n  private\n\n  protected\n\n  def bar; end\nend\n";
        let diags = run_cop_full_with_config(&AccessModifierDeclarations, source, inline_config());
        assert_eq!(
            diags.len(),
            1,
            "should only flag the bare modifier that has following defs (protected), not private"
        );
        assert!(
            diags[0].message.contains("protected"),
            "offense should be on `protected`, not `private`"
        );
    }
}
