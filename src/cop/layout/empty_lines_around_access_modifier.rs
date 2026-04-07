use crate::cop::shared::access_modifier_predicates::{self, MacroScope};
use crate::cop::shared::util::is_blank_or_whitespace_line;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;

/// Layout/EmptyLinesAroundAccessModifier
///
/// Investigation findings (2026-03-11):
///
/// FP root causes:
/// 1. Visitor did not exclude `def`/`defs` bodies — any `private` call inside a
///    method body in a class was incorrectly collected as an access modifier.
///    Fix: added `visit_def_node` to set `in_class_body = false`.
/// 2. Multiline class/module definitions (`class Foo <\n  Bar`) were not recognized
///    as body openings. The text-based `is_body_opening` only checked if the previous
///    line started with `class`/`module`, missing the continuation line.
///    Fix: store class/module/block opening lines from the AST in the collector, and
///    use those for boundary detection instead of text matching.
/// 3. Whitespace-only "blank" lines (e.g., lines with trailing spaces/tabs) were
///    not recognized as blank by `is_blank_line`. Repos like coderay and redcar use
///    trailing whitespace on otherwise empty lines.
///    Fix: switched to `is_blank_or_whitespace_line` (2026-03-14).
///
/// FN root causes:
/// 1. Access modifiers with trailing comments (`private # comment`) were rejected by
///    the line-content check which required `end_trimmed == method_name`.
///    Fix: allow trailing `# comment` after the modifier.
/// 2. Access modifiers inside macro blocks (`included do ... end`) were excluded by the
///    visitor (pushed as non-class scope), but RuboCop treats receiverless macro
///    blocks and class-constructor blocks as valid scopes.
///    Fix: treat receiverless calls in macro scope and `Class.new` / `Module.new`
///    style constructors as scope openers, while generic nested blocks inside
///    method bodies remain excluded (2026-03-14, refined 2026-03-15).
/// 3. Bare top-level access modifiers were never collected because the visitor
///    treated an empty scope stack as "outside a macro scope". RuboCop's
///    `in_macro_scope?` explicitly includes the file root, so `public`/`private`
///    at top level were missed, including `private` followed by a comment line.
///    Fix: treat the root as a valid access-modifier scope boundary while still
///    requiring explicit block propagation for nested scopes (2026-03-15).
///
/// 4. Remaining verifier gaps came from wrapper semantics around receiverful
///    blocks. RuboCop's `bare_access_modifier?` treats receiverful blocks as
///    valid wrappers once execution is already inside a non-root macro scope
///    (e.g. `1.times { private }`, `module_eval { module_function }`, and
///    nested `Builder.new do ... end` inside a class-scoped DSL block), but it
///    breaks propagation through local-variable assignment and explicit
///    `begin/rescue` wrappers. Fix: keep receiverful blocks active only when
///    the current scope is `ClassLike` or `DslBlock`, reset scope for local
///    variable writes, and treat `BeginNode` with `rescue`/`ensure` as a scope
///    break (2026-03-15, round 3). Inline brace-block forms like
///    `1.times { private }` also need to bypass the old whole-line
///    `is_bare_modifier_line` filter.
///
/// 5. `is_inline_brace_block_modifier_line` was too broad: any line containing
///    `{` before the modifier column and `}` after matched, even hash literals
///    (`{id: public.id}`) and multi-statement inline blocks
///    (`Class.new{ private; def foo; end }`). Fix: require the modifier to be
///    the SOLE content between `{` and `}`, ignoring whitespace (2026-03-15).
/// 6. Receiverful blocks at Root scope (e.g., `Puma::Plugin.create do ... end`)
///    were pushed as `NonClass`, so bare access modifiers inside were missed.
///    RuboCop's `in_macro_scope?` treats any block whose parent is in macro
///    scope (including root) as valid. Fix: allow `Root` in the receiverful
///    block scope check (2026-03-15).
///
/// Updated status (2026-03-15, round 4):
/// - `verify-cop-locations.py` reports ALL 14 known CI FP/FN (9 FP, 5 FN)
///   fixed locally for this cop.
/// - `check-cop.py --verbose --rerun` reports `Missing=0`, with excess
///   offense total within existing file-drop-noise from parser-crash repos.
///
/// 7. FP from jruby: `defined?(PTY) and ... and TestIO_Console.class_eval do`
///    wraps the block in an `AndNode`. RuboCop's `in_macro_scope?` only
///    recognizes `kwbegin`, `begin`, `any_block`, and `if` as valid wrapper
///    nodes — `and`/`or` break the macro-scope chain, so
///    `bare_access_modifier?` returns false and the cop doesn't fire.
///    Fix: push `NonClass` scope for `AndNode`/`OrNode` so blocks nested
///    inside boolean combinators are not treated as macro scopes (2026-03-16).
///
/// 8. Comment lines after a class/module opening were still treated as part of
///    the opening-line exemption because the previous-line scan accepted any
///    earlier body-opening line as a separator. RuboCop only grants that
///    exemption when the modifier is on the immediate next line after the
///    opening. Fix: rely on the explicit body-opening line check and stop
///    treating older body-opening lines as blank separators (2026-03-30).
///
/// 9. `body_end?` in RuboCop only applies to class/module/sclass bodies, not to
///    generic macro blocks, and its boundary tracking is overwritten by nested
///    class-like definitions visited earlier in the same outer body. Fix: only
///    apply the closing-boundary exemption to root/class-like scopes, and
///    disable it for a class-like scope once a nested class/module/sclass has
///    been visited before the modifier (2026-03-30).
///
/// 10. Inline body-opening forms like `module Backend; private` and
///     `module Utils module_function` are still bare access modifiers in
///     RuboCop as long as everything from the selector to end-of-line is just
///     the selector plus optional whitespace/comment. Fix: validate the trailing
///     slice starting at the selector column instead of requiring the whole line
///     to contain only the modifier (2026-03-30).
///
/// 11. Remaining corpus false positives fell into three RuboCop-compatibility
///     gaps: we treated non-statement uses like `if public` / `eq public` as
///     bare modifiers, we let `case` / `when` propagate macro scope into
///     receiverful `class_eval` blocks, and we only honored the enclosing
///     scope's opening line instead of RuboCop's last-seen class/block opening
///     markers. Fix: require the call to sit in a direct body-statement
///     position, push `NonClass` through `case` / `case in`, and capture the
///     latest visited class/block opening lines for the blank-before exemption
///     (2026-04-02).
///
/// 12. The remaining false negatives were class-constructor blocks nested inside
///     larger expressions such as `Class.new do ... end.new`,
///     `include(Module.new do ... end)`, and
///     `prepend Module.new { ... }`. We already promoted those blocks to
///     class-like scope, but the surrounding `expression_depth` leaked into the
///     constructor block body, so bare `private`/`protected`/`public` calls were
///     still filtered out. Fix: reset `expression_depth` while visiting a
///     class-constructor block body so its statements are checked like a normal
///     class/module body (2026-04-03).
///
/// 13. The `only_before` variant was only checking `has_blank_before` and never
///     checked `has_blank_after`. RuboCop's `only_before` style flags a blank
///     line AFTER the modifier for `private`/`protected` (special_modifiers),
///     telling users to remove it. It does not check blank lines after
///     `module_function` or `public`. Fix: added `has_blank_after` check to the
///     `only_before` branch for special_modifiers only, reporting "Remove a blank
///     line after `{modifier}`." when a blank line follows (2026-04-06).
///
/// 14. Blocks inside class bodies (e.g. `setup do...end`, `included do...end`)
///     were incorrectly calling `note_nested_class_like()`, which disabled the
///     `body_end_boundary` exemption. This caused 6 default-style FPs where a
///     modifier right before `end` (with a blank before) was flagged as missing
///     a blank after. Fix: removed the `note_nested_class_like()` call from the
///     block handler in `visit_call_node` — only actual class/module/sclass
///     definitions should disable body_end_boundary (2026-04-07).
///
/// 15. The `only_before` style used `has_blank_after` (which includes the
///     body-end exemption) to decide whether to flag "Remove blank line after."
///     This caused 78 FPs where a special modifier before `end` (no actual
///     blank line) was incorrectly flagged. Fix: rewrote the `only_before`
///     branch to match RuboCop's `allowed_only_before_style?` exactly:
///     (a) exempt when the next line is literal `end` (no indentation),
///     (b) use `next_line_empty_and_exists?` (body_end OR actual blank, AND
///     not at EOF) for the "Remove after" check, and (c) fire at most one
///     offense per modifier (2026-04-07).
pub struct EmptyLinesAroundAccessModifier;

// Uses access_modifier_predicates for access modifier detection.

/// Check if a line is a comment (first non-whitespace character is `#`).
fn is_comment_line(line: &[u8]) -> bool {
    for &b in line {
        if b == b' ' || b == b'\t' {
            continue;
        }
        return b == b'#';
    }
    false
}

/// Check whether the source from `column` to end-of-line is exactly the access
/// modifier keyword, optionally followed by whitespace and a trailing comment.
/// This matches plain `private`, `private # comment`, and same-line body-opening
/// forms such as `module Backend; private # comment`.
fn is_trailing_bare_modifier_line(line: &[u8], column: usize, method_name: &[u8]) -> bool {
    let end_pos = column.saturating_add(method_name.len());
    if end_pos > line.len() || &line[column..end_pos] != method_name {
        return false;
    }

    let mut idx = end_pos;
    while idx < line.len() {
        match line[idx] {
            b' ' | b'\t' | b'\r' | b'\n' => idx += 1,
            b'#' => return true,
            _ => return false,
        }
    }

    true
}

/// Allow inline brace-block forms like `1.times { private }` and
/// `module_eval { module_function }`, which RuboCop still treats as bare
/// access modifiers even though the line contains surrounding block syntax.
/// The modifier must be the SOLE content between `{` and `}` (ignoring
/// whitespace). This prevents matching hash literals like `{id: public.id}`
/// and multi-statement inline blocks like `Class.new{ private; def foo; end }`.
fn is_inline_brace_block_modifier_line(line: &[u8], column: usize, method_name: &[u8]) -> bool {
    let end_pos = column.saturating_add(method_name.len());
    if end_pos > line.len() {
        return false;
    }

    if &line[column..end_pos] != method_name {
        return false;
    }

    let before = &line[..column];
    let after = &line[end_pos..];

    // Find the last `{` before the modifier
    let Some(brace_pos) = before.iter().rposition(|&b| b == b'{') else {
        return false;
    };
    // Everything between `{` and the modifier must be whitespace
    let between_open_and_mod = &before[brace_pos + 1..];
    if !between_open_and_mod
        .iter()
        .all(|&b| b == b' ' || b == b'\t')
    {
        return false;
    }

    // Find the first `}` after the modifier
    let Some(close_pos) = after.iter().position(|&b| b == b'}') else {
        return false;
    };
    // Everything between the modifier and `}` must be whitespace
    let between_mod_and_close = &after[..close_pos];
    between_mod_and_close
        .iter()
        .all(|&b| b == b' ' || b == b'\t')
}

/// Collected access modifier with context about its enclosing scope.
struct ModifierInfo {
    /// Byte offset of the access modifier call.
    offset: usize,
    /// Byte offset of the body opening of the enclosing class/module/block.
    /// For `class Foo < Bar`, this is `Bar`'s location start. For `class Foo`,
    /// this is the `class` keyword offset. For blocks, this is the block opening.
    body_opening_line: usize,
    /// Byte offset of the end of the enclosing class/module/block.
    body_closing_line: usize,
    /// Whether this modifier should treat the line before the closing `end` as a
    /// valid blank-after boundary.
    body_end_boundary: bool,
    /// Last class/module/sclass opening offset visited before this modifier.
    last_class_like_opening_line: Option<usize>,
    /// Last block opening offset visited before this modifier.
    last_block_opening_line: Option<usize>,
}

/// Whether this scope was entered as a root or class-like context (needed
/// for body_end_boundary which only applies to Root/ClassLike, not DslBlock).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeOrigin {
    Root,
    ClassLike,
    Other,
}

/// AST visitor that collects byte offsets of bare access modifier calls that are
/// direct children of class/module/singleton_class bodies (not method or lambda bodies).
struct AccessModifierCollector {
    /// Collected access modifier info.
    modifiers: Vec<ModifierInfo>,
    /// Stack of scope state for access-modifier tracking.
    scope_stack: Vec<ScopeState>,
    /// Non-zero while visiting nested expression positions (call args/receivers,
    /// conditional predicates) that cannot contain bare access modifiers.
    expression_depth: usize,
    /// RuboCop tracks the most recently visited class/module/sclass opening line
    /// globally, not per enclosing scope.
    last_class_like_opening_line: Option<usize>,
    /// RuboCop also tracks the most recently visited block opening line globally.
    last_block_opening_line: Option<usize>,
}

struct ScopeState {
    macro_scope: MacroScope,
    origin: ScopeOrigin,
    body_opening_line: usize,
    body_closing_line: usize,
    seen_nested_class_like: bool,
}

impl AccessModifierCollector {
    fn in_access_modifier_scope(&self) -> bool {
        self.scope_stack
            .last()
            .map(|scope| scope.macro_scope.is_macro())
            .unwrap_or(false)
    }

    fn current_scope(&self) -> (usize, usize, bool) {
        self.scope_stack
            .last()
            .map(|scope| {
                (
                    scope.body_opening_line,
                    scope.body_closing_line,
                    matches!(scope.origin, ScopeOrigin::Root)
                        || matches!(scope.origin, ScopeOrigin::ClassLike)
                            && !scope.seen_nested_class_like,
                )
            })
            .unwrap_or((0, 0, false))
    }

    fn current_macro_scope(&self) -> MacroScope {
        self.scope_stack
            .last()
            .map(|scope| scope.macro_scope)
            .unwrap_or(MacroScope::InMacroScope)
    }

    fn check_call(&mut self, call: &ruby_prism::CallNode<'_>) {
        if !self.in_access_modifier_scope() {
            return;
        }
        if self.expression_depth > 0 {
            return;
        }
        if !access_modifier_predicates::is_bare_access_modifier(call) {
            return;
        }
        let (body_opening_line, body_closing_line, body_end_boundary) = self.current_scope();
        self.modifiers.push(ModifierInfo {
            offset: call.location().start_offset(),
            body_opening_line,
            body_closing_line,
            body_end_boundary,
            last_class_like_opening_line: self.last_class_like_opening_line,
            last_block_opening_line: self.last_block_opening_line,
        });
    }

    fn push_class_scope(&mut self, body_opening_line: usize, body_closing_line: usize) {
        self.scope_stack.push(ScopeState {
            macro_scope: MacroScope::InMacroScope,
            origin: ScopeOrigin::ClassLike,
            body_opening_line,
            body_closing_line,
            seen_nested_class_like: false,
        });
    }

    fn push_wrapper_scope(&mut self, body_opening_line: usize, body_closing_line: usize) {
        self.scope_stack.push(ScopeState {
            macro_scope: self.current_macro_scope(),
            origin: ScopeOrigin::Other,
            body_opening_line,
            body_closing_line,
            seen_nested_class_like: false,
        });
    }

    fn push_non_class_scope(&mut self) {
        self.scope_stack.push(ScopeState {
            macro_scope: MacroScope::NotMacroScope,
            origin: ScopeOrigin::Other,
            body_opening_line: 0,
            body_closing_line: 0,
            seen_nested_class_like: false,
        });
    }

    fn note_nested_class_like(&mut self) {
        for scope in self.scope_stack.iter_mut().rev() {
            if matches!(scope.origin, ScopeOrigin::ClassLike) {
                scope.seen_nested_class_like = true;
                break;
            }
        }
    }

    fn pop_scope(&mut self) {
        self.scope_stack.pop();
    }

    fn visit_class_constructor_block_body(&mut self, block: &ruby_prism::BlockNode<'_>) {
        let previous_expression_depth = std::mem::replace(&mut self.expression_depth, 0);
        ruby_prism::visit_block_node(self, block);
        self.expression_depth = previous_expression_depth;
    }
}

macro_rules! visit_write_node_as_non_class_scope {
    ($method:ident, $node_ty:ty, $visit_fn:ident) => {
        fn $method(&mut self, node: &$node_ty) {
            self.push_non_class_scope();
            ruby_prism::$visit_fn(self, node);
            self.pop_scope();
        }
    };
}

fn is_class_constructor_call(call: &ruby_prism::CallNode<'_>) -> bool {
    if call.name().as_slice() != b"new" {
        return false;
    }

    let Some(receiver) = call.receiver() else {
        return false;
    };

    if let Some(const_read) = receiver.as_constant_read_node() {
        return matches!(
            const_read.name().as_slice(),
            b"Class" | b"Module" | b"Struct" | b"Data"
        );
    }

    if let Some(const_path) = receiver.as_constant_path_node() {
        if const_path.parent().is_none() {
            if let Some(name_node) = const_path.name() {
                return matches!(
                    name_node.as_slice(),
                    b"Class" | b"Module" | b"Struct" | b"Data"
                );
            }
        }
    }

    false
}

impl<'pr> ruby_prism::Visit<'pr> for AccessModifierCollector {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        self.note_nested_class_like();
        // For multiline class definitions like `class Foo <\n  Bar`,
        // the body opening line is the parent class's line (where Bar is).
        // For simple `class Foo`, it's the class keyword line.
        let opening_line = if let Some(superclass) = node.superclass() {
            superclass.location().start_offset()
        } else {
            node.location().start_offset()
        };
        let closing_line = node.location().end_offset();
        self.last_class_like_opening_line = Some(opening_line);
        self.push_class_scope(opening_line, closing_line);
        ruby_prism::visit_class_node(self, node);
        self.pop_scope();
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        self.note_nested_class_like();
        let opening = node.location().start_offset();
        let closing = node.location().end_offset();
        self.last_class_like_opening_line = Some(opening);
        self.push_class_scope(opening, closing);
        ruby_prism::visit_module_node(self, node);
        self.pop_scope();
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        self.note_nested_class_like();
        // For `class << self`, the expression is `self` — use its line as opening.
        let opening = node.expression().location().start_offset();
        let closing = node.location().end_offset();
        self.last_class_like_opening_line = Some(opening);
        self.push_class_scope(opening, closing);
        ruby_prism::visit_singleton_class_node(self, node);
        self.pop_scope();
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        // Method bodies are not macro scopes — exclude them.
        self.push_non_class_scope();
        ruby_prism::visit_def_node(self, node);
        self.pop_scope();
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        self.last_block_opening_line = Some(node.location().start_offset());
        ruby_prism::visit_block_node(self, node);
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        if node.rescue_clause().is_some() || node.ensure_clause().is_some() {
            self.push_non_class_scope();
            ruby_prism::visit_begin_node(self, node);
            self.pop_scope();
            return;
        }

        ruby_prism::visit_begin_node(self, node);
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        self.last_block_opening_line = Some(node.location().start_offset());
        self.push_non_class_scope();
        ruby_prism::visit_lambda_node(self, node);
        self.pop_scope();
    }

    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        self.push_non_class_scope();
        ruby_prism::visit_rescue_node(self, node);
        self.pop_scope();
    }

    fn visit_ensure_node(&mut self, node: &ruby_prism::EnsureNode<'pr>) {
        self.push_non_class_scope();
        ruby_prism::visit_ensure_node(self, node);
        self.pop_scope();
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);

        if let Some(receiver) = node.receiver() {
            self.expression_depth += 1;
            self.visit(&receiver);
            self.expression_depth -= 1;
        }
        if let Some(arguments) = node.arguments() {
            self.expression_depth += 1;
            self.visit_arguments_node(&arguments);
            self.expression_depth -= 1;
        }

        if let Some(block_node) = node.block().and_then(|b| b.as_block_node()) {
            let opening = block_node.location().start_offset();
            let closing = block_node.location().end_offset();
            self.last_block_opening_line = Some(opening);

            if is_class_constructor_call(node) {
                self.push_class_scope(opening, closing);
                self.visit_class_constructor_block_body(&block_node);
                self.pop_scope();
                return;
            }

            // Blocks in macro scope inherit it (wrapper); otherwise non-macro.
            if self.in_access_modifier_scope() {
                self.push_wrapper_scope(opening, closing);
            } else {
                self.push_non_class_scope();
            }
            ruby_prism::visit_block_node(self, &block_node);
            self.pop_scope();
            return;
        }

        if let Some(block_arg) = node.block().and_then(|b| b.as_block_argument_node()) {
            self.expression_depth += 1;
            self.visit_block_argument_node(&block_arg);
            self.expression_depth -= 1;
        }
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        self.expression_depth += 1;
        self.visit(&node.predicate());
        self.expression_depth -= 1;

        if let Some(stmts) = node.statements() {
            self.visit_statements_node(&stmts);
        }
        if let Some(subsequent) = node.subsequent() {
            self.visit(&subsequent);
        }
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        self.expression_depth += 1;
        self.visit(&node.predicate());
        self.expression_depth -= 1;

        if let Some(stmts) = node.statements() {
            self.visit_statements_node(&stmts);
        }
        if let Some(else_clause) = node.else_clause() {
            self.visit_else_node(&else_clause);
        }
    }

    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'pr>) {
        // `case` / `when` are not wrappers in RuboCop's `in_macro_scope?`.
        self.push_non_class_scope();
        ruby_prism::visit_case_node(self, node);
        self.pop_scope();
    }

    fn visit_case_match_node(&mut self, node: &ruby_prism::CaseMatchNode<'pr>) {
        // `case` / `in` is also not a wrapper in RuboCop's `in_macro_scope?`.
        self.push_non_class_scope();
        ruby_prism::visit_case_match_node(self, node);
        self.pop_scope();
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        self.push_non_class_scope();
        ruby_prism::visit_and_node(self, node);
        self.pop_scope();
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        self.push_non_class_scope();
        ruby_prism::visit_or_node(self, node);
        self.pop_scope();
    }

    visit_write_node_as_non_class_scope!(
        visit_local_variable_write_node,
        ruby_prism::LocalVariableWriteNode<'pr>,
        visit_local_variable_write_node
    );
    visit_write_node_as_non_class_scope!(
        visit_local_variable_and_write_node,
        ruby_prism::LocalVariableAndWriteNode<'pr>,
        visit_local_variable_and_write_node
    );
    visit_write_node_as_non_class_scope!(
        visit_local_variable_operator_write_node,
        ruby_prism::LocalVariableOperatorWriteNode<'pr>,
        visit_local_variable_operator_write_node
    );
    visit_write_node_as_non_class_scope!(
        visit_local_variable_or_write_node,
        ruby_prism::LocalVariableOrWriteNode<'pr>,
        visit_local_variable_or_write_node
    );
}

impl Cop for EmptyLinesAroundAccessModifier {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundAccessModifier"
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let enforced_style = config.get_str("EnforcedStyle", "around");

        // Collect access modifier offsets that are in class/module bodies
        let mut collector = AccessModifierCollector {
            modifiers: Vec::new(),
            scope_stack: vec![ScopeState {
                macro_scope: MacroScope::InMacroScope,
                origin: ScopeOrigin::Root,
                body_opening_line: 0,
                body_closing_line: 0,
                seen_nested_class_like: false,
            }],
            expression_depth: 0,
            last_class_like_opening_line: None,
            last_block_opening_line: None,
        };
        use ruby_prism::Visit;
        collector.visit(&parse_result.node());

        let lines: Vec<&[u8]> = source.lines().collect();

        for modifier in &collector.modifiers {
            let (line, col) = source.offset_to_line_col(modifier.offset);

            // Determine the method name from the source at this offset
            let bytes = source.as_bytes();
            let method_name = access_modifier_predicates::ACCESS_MODIFIER_NAMES
                .iter()
                .find(|&&m| {
                    modifier.offset + m.len() <= bytes.len()
                        && &bytes[modifier.offset..modifier.offset + m.len()] == m
                });
            let method_name = match method_name {
                Some(m) => *m,
                None => continue,
            };

            // Ensure the access modifier is the only thing on its line (optionally with comment)
            if line > 0 && line <= lines.len() {
                let current_line = lines[line - 1];
                if !is_trailing_bare_modifier_line(current_line, col, method_name)
                    && !is_inline_brace_block_modifier_line(current_line, col, method_name)
                {
                    continue;
                }
            }

            let modifier_str = std::str::from_utf8(method_name).unwrap_or("");

            let at_root = modifier.body_opening_line == 0 && modifier.body_closing_line == 0;

            // Convert body opening/closing offsets to 1-based line numbers.
            let body_opening_line = if at_root {
                0
            } else {
                source.offset_to_line_col(modifier.body_opening_line).0
            };
            let body_closing_line = if at_root {
                lines.len() + 1
            } else {
                let body_closing_offset = modifier.body_closing_line;
                // The closing offset points to the end of `end`, so the `end` keyword is on
                // the line containing that offset. We want the line before that.
                if body_closing_offset > 0 {
                    let (cl, _) = source.offset_to_line_col(body_closing_offset - 1);
                    cl
                } else {
                    0
                }
            };

            // Check if we're at a class/module body opening (line right after the opening)
            let is_at_body_opening = line == body_opening_line + 1;
            let is_after_recent_class_like_opening = modifier
                .last_class_like_opening_line
                .map(|offset| source.offset_to_line_col(offset).0)
                .is_some_and(|opening_line| line == opening_line + 1);
            let is_after_recent_block_opening = modifier
                .last_block_opening_line
                .map(|offset| source.offset_to_line_col(offset).0)
                .is_some_and(|opening_line| line == opening_line + 1);

            // Check if we're at a body end (line right before the closing `end`)
            let is_at_body_end = modifier.body_end_boundary && line == body_closing_line - 1;
            let is_before_scope_closing_end =
                body_closing_line > 0 && line == body_closing_line - 1;

            // Find the previous non-comment line
            let has_blank_before = {
                if is_at_body_opening
                    || is_after_recent_class_like_opening
                    || is_after_recent_block_opening
                {
                    true
                } else {
                    let mut found_blank_or_boundary = true;
                    let mut idx = line as isize - 2;
                    while idx >= 0 {
                        let prev = lines[idx as usize];
                        if is_comment_line(prev) {
                            idx -= 1;
                            continue;
                        }
                        found_blank_or_boundary = is_blank_or_whitespace_line(prev);
                        break;
                    }
                    found_blank_or_boundary
                }
            };

            // Check blank line after
            let has_blank_after = if is_at_body_end {
                true
            } else if line < lines.len() {
                let next = lines[line];
                is_blank_or_whitespace_line(next)
            } else {
                true
            };

            match enforced_style {
                "around" => {
                    if !has_blank_before || !has_blank_after {
                        let msg = if (modifier.body_end_boundary || !is_before_scope_closing_end)
                            && has_blank_before
                            && !has_blank_after
                        {
                            format!("Keep a blank line after `{modifier_str}`.")
                        } else {
                            format!("Keep a blank line before and after `{modifier_str}`.")
                        };
                        let mut diag = self.diagnostic(source, line, col, msg);
                        if let Some(ref mut corr) = corrections {
                            if !has_blank_before {
                                if let Some(off) = source.line_col_to_offset(line, 0) {
                                    corr.push(crate::correction::Correction {
                                        start: off,
                                        end: off,
                                        replacement: "\n".to_string(),
                                        cop_name: self.name(),
                                        cop_index: 0,
                                    });
                                    diag.corrected = true;
                                }
                            }
                            if !has_blank_after {
                                if let Some(off) = source.line_col_to_offset(line + 1, 0) {
                                    corr.push(crate::correction::Correction {
                                        start: off,
                                        end: off,
                                        replacement: "\n".to_string(),
                                        cop_name: self.name(),
                                        cop_index: 0,
                                    });
                                    diag.corrected = true;
                                }
                            }
                        }
                        diagnostics.push(diag);
                    }
                }
                "only_before" => {
                    // Match RuboCop's allowed_only_before_style? logic:
                    // 1. Special modifier + next line is bare 'end' → no offense
                    // 2. Special modifier + next_line_empty_and_exists → "Remove after"
                    // 3. No blank before → "Keep before"
                    // RuboCop fires at most ONE offense per modifier.
                    let is_special = access_modifier_predicates::is_special_modifier_name(
                        modifier_str.as_bytes(),
                    );

                    if is_special {
                        // RuboCop: return true if processed_source[node.last_line] == 'end'
                        let next_line_is_bare_end = line < lines.len() && lines[line] == b"end";
                        if next_line_is_bare_end {
                            // No offense for this modifier
                            continue;
                        }

                        // RuboCop: return false if next_line_empty_and_exists?
                        // next_line_empty = body_end? || next_line.blank?
                        let next_empty = is_at_body_end
                            || (line < lines.len() && is_blank_or_whitespace_line(lines[line]));
                        // exists = modifier is not on the second-to-last line
                        let exists = line + 1 != lines.len();

                        if next_empty && exists {
                            let mut diag = self.diagnostic(
                                source,
                                line,
                                col,
                                format!("Remove a blank line after `{modifier_str}`."),
                            );
                            if let Some(ref mut corr) = corrections {
                                if let Some(off) = source.line_col_to_offset(line + 1, 0) {
                                    corr.push(crate::correction::Correction {
                                        start: off,
                                        end: off,
                                        replacement: String::new(),
                                        cop_name: self.name(),
                                        cop_index: 0,
                                    });
                                    diag.corrected = true;
                                }
                            }
                            diagnostics.push(diag);
                            continue;
                        }
                    }

                    if !has_blank_before {
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            col,
                            format!("Keep a blank line before `{modifier_str}`."),
                        );
                        if let Some(ref mut corr) = corrections {
                            if let Some(off) = source.line_col_to_offset(line, 0) {
                                corr.push(crate::correction::Correction {
                                    start: off,
                                    end: off,
                                    replacement: "\n".to_string(),
                                    cop_name: self.name(),
                                    cop_index: 0,
                                });
                                diag.corrected = true;
                            }
                        }
                        diagnostics.push(diag);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;
    use std::collections::HashMap;

    crate::cop_fixture_tests!(
        EmptyLinesAroundAccessModifier,
        "cops/layout/empty_lines_around_access_modifier"
    );
    crate::cop_autocorrect_fixture_tests!(
        EmptyLinesAroundAccessModifier,
        "cops/layout/empty_lines_around_access_modifier"
    );

    #[test]
    fn flags_only_before_style_with_blank_line_after() {
        let mut opts = HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("only_before".to_string()),
        );
        let config = CopConfig {
            options: opts,
            ..CopConfig::default()
        };
        let diags = crate::testutil::run_cop_full_with_config(
            &EmptyLinesAroundAccessModifier,
            b"class Test\n  def bar; end\n\n  private\n\n  def baz; end\nend\n",
            config,
        );
        // only_before style: blank line AFTER is an offense (should be removed)
        assert_eq!(diags.len(), 1, "Expected 1 offense but got {:?}", diags);
        assert_eq!(diags[0].location.line, 4);
        assert!(diags[0].message.contains("Remove a blank line after"));
    }

    #[test]
    fn flags_only_before_style_missing_blank_before() {
        let mut opts = HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("only_before".to_string()),
        );
        let config = CopConfig {
            options: opts,
            ..CopConfig::default()
        };
        let diags = crate::testutil::run_cop_full_with_config(
            &EmptyLinesAroundAccessModifier,
            b"class Test\n  def bar; end\n  private\n  def baz; end\nend\n",
            config,
        );
        // only_before style: missing blank line BEFORE is an offense
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 3);
        assert!(diags[0].message.contains("Keep a blank line before"));
    }

    #[test]
    fn only_before_style_allows_blank_line_after_module_function() {
        // module_function is NOT a special_modifier (only private/protected are),
        // so only_before style should NOT flag blank line after module_function.
        // This is a regression test for FP in module_function repos.
        let mut opts = HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("only_before".to_string()),
        );
        let config = CopConfig {
            options: opts,
            ..CopConfig::default()
        };
        let diags = crate::testutil::run_cop_full_with_config(
            &EmptyLinesAroundAccessModifier,
            b"module Test\n  module_function\n\n  def foo; end\nend\n",
            config,
        );
        // no offense - only_before doesn't check blank line after module_function
        assert_eq!(diags.len(), 0);
    }

    #[test]
    fn flags_bare_modifier_inside_receiverful_block_in_class_scope() {
        let diags = run_cop_full(
            &EmptyLinesAroundAccessModifier,
            b"class TestVis2\n  public\n\n  def foo; end\n\n  1.times { private }\n  def foo; end\n\n  1.times { public }\n  def foo; end\nend\n",
        );

        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].location.line, 6);
        assert_eq!(diags[1].location.line, 9);
    }

    #[test]
    fn flags_module_function_inside_module_eval_within_class_constructor() {
        let diags = run_cop_full(
            &EmptyLinesAroundAccessModifier,
            b"it do\n  m = Module.new do\n    module_eval { module_function }\n    def test1() end\n    def test2() end\n  end\nend\n",
        );

        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 3);
    }

    #[test]
    fn flags_modifier_inside_class_constructor_chained_with_new() {
        let diags = run_cop_full(
            &EmptyLinesAroundAccessModifier,
            b"class Test\n  def build\n    obj = Class.new do\n      private\n        def private_property\n        end\n\n      protected\n        def protected_property\n        end\n    end.new\n  end\nend\n",
        );

        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].location.line, 4);
        assert_eq!(diags[1].location.line, 8);
    }

    #[test]
    fn flags_modifier_inside_module_constructor_passed_as_argument() {
        let diags = run_cop_full(
            &EmptyLinesAroundAccessModifier,
            b"class Wrapper\n  include(Module.new do\n    private\n    def helper\n    end\n  end)\nend\n",
        );

        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 3);
    }

    #[test]
    fn flags_modifier_inside_brace_module_constructor_passed_as_argument() {
        let diags = run_cop_full(
            &EmptyLinesAroundAccessModifier,
            b"Runner.singleton_class.prepend Module.new {\n  private\n    def list_tests\n    end\n}\n",
        );

        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 2);
    }

    #[test]
    fn only_before_block_in_class_counts_as_nested_content() {
        // A block inside a class body should count as nested content, so
        // a modifier after the block with a blank line before it is correct
        // in only_before style (no FP).
        let mut opts = HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("only_before".to_string()),
        );
        let config = CopConfig {
            options: opts,
            ..CopConfig::default()
        };
        let diags = crate::testutil::run_cop_full_with_config(
            &EmptyLinesAroundAccessModifier,
            b"class Foo\n  some_method do\n    something\n  end\n\n  private\n\n  def baz; end\nend\n",
            config,
        );
        // only_before: blank after private is the only offense
        assert_eq!(diags.len(), 1, "Expected 1 offense but got {:?}", diags);
        assert!(
            diags[0].message.contains("Remove a blank line after"),
            "Expected blank-after offense, got: {}",
            diags[0].message
        );
    }

    #[test]
    fn ignores_receiverful_block_inside_rescue_wrapper() {
        let diags = run_cop_full(
            &EmptyLinesAroundAccessModifier,
            b"report \"loading program\" do\n  begin\n    require \"jaro_winkler\"\n    DidYouMean::JaroWinkler.module_eval do\n      module_function\n      def distance(str1, str2)\n      end\n    end\n  rescue LoadError\n  end\nend\n",
        );

        assert!(diags.is_empty());
    }

    #[test]
    fn no_offense_modifier_at_body_end_after_block() {
        // Bug 1 regression: blocks in class scope (like `included do...end`,
        // `setup do...end`) should NOT disable body_end_boundary. A modifier
        // right before `end` with a blank line before it is valid.
        let diags = run_cop_full(
            &EmptyLinesAroundAccessModifier,
            b"module Loggable\n  included do\n    has_many :outputs\n  end\n\n  def info(line)\n  end\n\n  private\nend\n",
        );
        assert!(
            diags.is_empty(),
            "Expected no offense for modifier at body end after block, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_modifier_at_body_end_after_setup_block() {
        // Regression test: `setup do...end` block should not disable body_end
        let diags = run_cop_full(
            &EmptyLinesAroundAccessModifier,
            b"class AppTest < ActionDispatch::IntegrationTest\n  setup do\n    do_something\n  end\n\n  protected\nend\n",
        );
        assert!(diags.is_empty(), "Expected no offense, got: {:?}", diags);
    }

    #[test]
    fn only_before_no_offense_modifier_before_bare_end() {
        // Bug 2 regression: only_before style should not flag "Remove blank
        // after" when the next line is bare `end` (no indentation).
        let mut opts = HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("only_before".to_string()),
        );
        let config = CopConfig {
            options: opts,
            ..CopConfig::default()
        };
        let diags = crate::testutil::run_cop_full_with_config(
            &EmptyLinesAroundAccessModifier,
            b"class Foo\n  def run; end\n\n  private\nend\n",
            config,
        );
        assert!(
            diags.is_empty(),
            "Expected no offense for only_before with bare end, got: {:?}",
            diags
        );
    }

    #[test]
    fn only_before_no_offense_modifier_at_body_end_after_block() {
        // Combines Bug 1 + Bug 2: block in class + only_before style +
        // modifier before bare `end` should be no offense.
        let mut opts = HashMap::new();
        opts.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("only_before".to_string()),
        );
        let config = CopConfig {
            options: opts,
            ..CopConfig::default()
        };
        let diags = crate::testutil::run_cop_full_with_config(
            &EmptyLinesAroundAccessModifier,
            b"module Loggable\n  included do\n    has_many :outputs\n  end\n\n  private\nend\n",
            config,
        );
        assert!(diags.is_empty(), "Expected no offense, got: {:?}", diags);
    }
}
