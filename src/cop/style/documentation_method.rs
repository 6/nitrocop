use crate::cop::shared::access_modifier_predicates;
use crate::cop::style::documentation::trim_bytes;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;
use std::collections::HashMap;

/// Modifiers that wrap a def on the same line but are considered non-public.
const NON_PUBLIC_MODIFIERS: &[&[u8]] = &[b"private_class_method "];

/// Modifiers that wrap a def on the same line but are still public.
/// Documentation should be checked above the modifier line, and the offense
/// reported at the modifier start.
const PUBLIC_MODIFIERS: &[&[u8]] = &[b"module_function ", b"ruby2_keywords "];

/// Style/DocumentationMethod: checks for missing documentation comment on public methods.
///
/// **Investigation (2026-03-08):** 1,768 FPs, 453 FNs at 99.5% match rate.
/// Root cause of FPs: retroactive visibility via `private :method_name` or
/// `protected :method_name` after the def. RuboCop's `VisibilityHelp` mixin checks
/// right-siblings for `(send nil? :private (sym :method_name))`, making the method
/// non-public. nitrocop's `is_private_or_protected` only checked preceding standalone
/// visibility keywords and inline prefixes — it missed the retroactive pattern.
///
/// Fix: Added `has_retroactive_visibility()` which scans lines after the def for
/// `private :sym` / `protected :sym` / `private "str"` patterns with matching method name.
///
/// **Investigation (2026-03-15):** 918 FPs, 508 FNs at 99.7% match rate.
/// Root cause of remaining FPs: three patterns in `is_private_or_protected`:
/// 1. Nested class/module at same indent as `private`/`def` incorrectly reset
///    `in_private` state. E.g., `private; class Inner; end; def method` — the
///    `class Inner` is a peer in the same scope, not a scope boundary. Fix: changed
///    class/module reset from `indent <= def_col` to `indent < def_col` (strictly less).
/// 2. Trailing whitespace on `private` line (e.g., `private \n`) was not matched by
///    bare visibility keyword patterns. Fix: strip trailing whitespace from trimmed lines.
/// 3. `private(def foo)` and `protected(def foo)` — parenthesized form not recognized
///    as inline visibility modifier. Fix: added `private(` / `protected(` patterns to
///    both `is_private_or_protected` and `has_unknown_inline_prefix`.
///
/// Remaining FN gap (508): singleton methods (`def self.foo`, `def obj.bar`) not handled.
/// RuboCop uses `on_defs` (aliased from `on_def`), nitrocop only handles `DefNode`.
///
/// **Investigation (2026-03-18):** 426 FPs, 383 FNs.
/// Root causes of remaining FPs in `is_private_or_protected`:
/// 1. Single-line class/module defs (`class Error < StandardError; end`) at indent ==
///    def_col were incrementing `peer_scope_depth` but never decrementing it (the `end`
///    is on the same line). This caused all subsequent `private` keywords to be ignored.
///    Fix: added `is_single_line_class_or_module()` check — skip peer_scope_depth++ when
///    the class/module opens and closes on the same line.
/// 2. Heredoc content containing `end` at column 0 (e.g., `buf.puts(<<-RUBY)\nend\nRUBY`)
///    was incorrectly treated as a scope boundary, resetting `in_private` state.
///    Fix: added heredoc tracking in `is_private_or_protected` — detect `<<-WORD` / `<<~WORD`
///    patterns and skip all lines until the closing heredoc marker.
/// 3. Some FPs from enclosing class at same indent as def (inconsistent indentation) —
///    inherent limitation of line-based visibility tracking vs RuboCop's AST approach.
///
/// **Heredoc tracking reverted (2026-03-18):** The heredoc tracking added in the previous
/// investigation caused a 20,000+ offense regression. Even with conservative `<<` matching
/// (skip comments, check preceding chars), the fix correctly detected real heredocs but
/// produced worse results: the line-based scanner incidentally processes heredoc content,
/// and `private`/`end` keywords inside heredocs happen to give correct visibility results
/// more often than skipping them. The single-line class/module fix (point 1 above) is
/// retained. A proper fix for heredoc-related FPs requires AST-based visibility tracking.
///
/// **Investigation (2026-04-01):** Mixed remaining FNs came from two method-specific gaps:
/// 1. `# Note:` above a method was treated as documentation because the shared annotation
///    filter only matched uppercase `NOTE`. RuboCop treats annotation keywords case-
///    insensitively, so `Note:` still suppresses documentation, but sentence-style
///    comments like `# Note that ...` and `# Note to ...` still count as documentation.
/// 2. `private`/`protected` visibility leaked through wrapper nodes because this cop used a
///    line-based scan. RuboCop's `VisibilityHelp` only looks at AST siblings in the same
///    statement list, so `private` before `if ...; def foo; end` does not make the branch
///    method non-public. Fix: use AST sibling visibility for this cop's standalone and
///    retroactive visibility checks, while keeping existing inline-modifier handling.
///    RuboCop's retroactive matcher is narrower than Ruby itself: it only treats
///    `private :foo` / `protected :foo` / `public :foo` with exactly one symbol
///    argument as visibility-changing for this cop. Calls with multiple symbols
///    (`protected :foo, :bar`) or strings (`private "foo"`) still require docs.
/// 3. Wrapper nodes still stole comments in two narrower cases that the line scan missed:
///    inline visibility calls like `public def foo` / `protected (def foo)` and postfix
///    modifiers like `def foo; end if false`. RuboCop associates comments above those lines
///    with the wrapping call/modifier node, not the inner `def`, so they must still be
///    reported as undocumented.
///
/// **Investigation (2026-04-04):** 94 FP, 63 FN.
/// Root causes of remaining FPs: `wrapped_comment_depth` and `inline_non_public_depth`
/// leaked through scope boundaries (class, module, def bodies). For example, a postfix
/// modifier like `class Hash ... end unless defined? ActiveSupport` incremented
/// `wrapped_comment_depth`, which then propagated into the class body, causing all
/// documented methods inside to be falsely flagged. Similarly, `private def wrapper` leaked
/// `inline_non_public_depth` into nested singleton method defs inside the wrapper body.
/// Fix: save/restore both counters at class, module, singleton class, and def node
/// boundaries so that nested scopes start fresh.
///
/// Root cause of some remaining FNs: comment lines with an inline `# rubocop:disable`
/// directive (e.g., `# Some doc # rubocop:disable Layout/LineLength`) were treated as
/// documentation by nitrocop but RuboCop's `DirectiveComment` matches `# rubocop:` anywhere
/// in the comment text, treating the entire line as a directive. Fix: check for
/// `# rubocop:` as a substring in the comment text.
///
/// **Investigation (2026-04-10):** 41 FP, 52 FN.
/// Two root causes addressed:
///
/// 1. FN: When `def` has a non-modifier prefix on the same line (e.g.,
///    `__skip__ = def foo`, `class << obj; def foo`, `Module.new { def foo`),
///    nitrocop's line-based documentation check found comments above the
///    outer expression and treated them as documentation for the inner def.
///    RuboCop's AST-based comment association correctly assigns those comments
///    to the outer node (assignment, singleton class, call), not the inner def.
///    Fix: added `has_non_modifier_prefix()` check — when the def has
///    non-whitespace content before it that isn't a known modifier
///    (module_function/ruby2_keywords), skip the documentation check so the
///    def is flagged as undocumented. Resolved 26 FN.
///
/// 2. FP: `ruby2_keywords def foo` and `module_function def foo` in a `private`
///    or `protected` section were incorrectly flagged because the inner def
///    didn't inherit the enclosing scope's preceding visibility. The statement
///    is a CallNode (not a DefNode), so `visit_statements_node` didn't register
///    it in `pending_visibility`. Fix: detect modifier calls wrapping DefNode
///    arguments and register the inner def with the enclosing visibility.
pub struct DocumentationMethod;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MethodVisibility {
    Private,
    Protected,
    Public,
}

/// Detect if the line containing the def has a modifier prefix before the `def` keyword.
/// Returns `Some((modifier_bytes, indent))` if found, where `indent` is the column of the
/// modifier's first non-whitespace character.
fn detect_inline_modifier(source: &SourceFile, def_offset: usize) -> Option<(&[u8], usize)> {
    let bytes = source.as_bytes();
    // Find the start of the line containing the def
    let mut line_start = def_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let line_prefix = &bytes[line_start..def_offset];

    // Compute indent (leading whitespace)
    let indent = line_prefix
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count();
    let trimmed = &line_prefix[indent..];

    // Check all known modifiers
    for modifier in NON_PUBLIC_MODIFIERS.iter().chain(PUBLIC_MODIFIERS.iter()) {
        if trimmed.starts_with(modifier) {
            return Some((modifier, indent));
        }
    }
    None
}

/// Check if the detected modifier is a non-public modifier.
fn is_non_public_modifier(modifier: &[u8]) -> bool {
    NON_PUBLIC_MODIFIERS.contains(&modifier)
}

/// Returns true if the def keyword has non-whitespace content before it on the
/// same line that is NOT a known modifier prefix. When true, comments above the
/// line belong to the outer expression (assignment, call, class << expr, etc.),
/// not the inner def — mirroring RuboCop's AST-based comment association.
fn has_non_modifier_prefix(source: &SourceFile, def_offset: usize) -> bool {
    let bytes = source.as_bytes();
    let mut line_start = def_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let prefix = &bytes[line_start..def_offset];
    prefix.iter().any(|&b| b != b' ' && b != b'\t')
}

/// Check if a method is made private/protected retroactively via a single-symbol
/// visibility call like `private :method_name` or `protected :method_name`
/// appearing after the def in the same scope.
fn visibility_name(node: &ruby_prism::Node<'_>) -> Option<MethodVisibility> {
    let call = node.as_call_node()?;
    if !access_modifier_predicates::is_access_modifier_declaration(&call) {
        return None;
    }
    match call.name().as_slice() {
        b"private" => Some(MethodVisibility::Private),
        b"protected" => Some(MethodVisibility::Protected),
        b"public" => Some(MethodVisibility::Public),
        _ => None,
    }
}

fn single_symbol_argument_matches_method_name(
    call: &ruby_prism::CallNode<'_>,
    method_name: &str,
) -> bool {
    let Some(args) = call.arguments() else {
        return false;
    };

    let mut args = args.arguments().iter();
    let Some(arg) = args.next() else {
        return false;
    };
    if args.next().is_some() {
        return false;
    }

    let Some(sym) = arg.as_symbol_node() else {
        return false;
    };

    std::str::from_utf8(sym.unescaped()).unwrap_or("") == method_name
}

fn retroactive_visibility_from_siblings(
    siblings: &[ruby_prism::Node<'_>],
    method_name: &str,
) -> Option<MethodVisibility> {
    for sibling in siblings.iter().rev() {
        let Some(vis) = visibility_name(sibling) else {
            continue;
        };
        let Some(call) = sibling.as_call_node() else {
            continue;
        };
        if single_symbol_argument_matches_method_name(&call, method_name) {
            return Some(vis);
        }
    }

    None
}

fn standalone_visibility(node: &ruby_prism::Node<'_>) -> Option<MethodVisibility> {
    let vis = visibility_name(node)?;
    let call = node.as_call_node()?;
    let has_args = call
        .arguments()
        .is_some_and(|args| !args.arguments().is_empty());
    if has_args { None } else { Some(vis) }
}

fn call_wrapper_steals_comments(call: &ruby_prism::CallNode<'_>) -> bool {
    call.receiver().is_none()
        && !matches!(
            call.name().as_slice(),
            b"module_function" | b"ruby2_keywords"
        )
}

fn inline_non_public_call(call: &ruby_prism::CallNode<'_>, arg: &ruby_prism::Node<'_>) -> bool {
    call.receiver().is_none()
        && matches!(call.name().as_slice(), b"private" | b"protected")
        && arg.as_def_node().is_some()
}

pub fn is_annotation_or_directive_case_insensitive(comment: &str) -> bool {
    let text = comment.trim_start_matches('#').trim();

    if text.starts_with("frozen_string_literal:")
        || text.starts_with("encoding:")
        || text.starts_with("coding:")
        || text.starts_with("warn_indent:")
        || text.starts_with("shareable_constant_value:")
        || text.starts_with("rubocop:")
    {
        return true;
    }

    // RuboCop's DirectiveComment matches `# rubocop:` anywhere in the comment text,
    // so a doc comment with an inline rubocop:disable (e.g., `# Some doc # rubocop:disable X`)
    // is treated as a directive, not documentation.
    if text.contains("# rubocop:") {
        return true;
    }

    for kw in ["TODO", "FIXME", "OPTIMIZE", "HACK", "REVIEW", "NOTE"] {
        // Use get() to avoid panicking on multi-byte UTF-8 chars at the boundary.
        // Keywords are ASCII-only, so a non-char-boundary means no match.
        let prefix = match text.get(..kw.len()) {
            Some(s) => s,
            None => continue,
        };
        if !prefix.eq_ignore_ascii_case(kw) {
            continue;
        }

        let rest = &text[kw.len()..];
        if !rest.is_empty()
            && !rest.starts_with(':')
            && !rest.starts_with(' ')
            && !rest.starts_with('\t')
        {
            continue;
        }

        let ws_prefix_len = rest
            .bytes()
            .take_while(|b| *b == b' ' || *b == b'\t')
            .count();
        let rest_after_ws = &rest[ws_prefix_len..];
        let colon = rest_after_ws.starts_with(':');
        let rest_after_colon = if colon {
            &rest_after_ws[1..]
        } else {
            rest_after_ws
        };
        let space_after = rest_after_colon
            .bytes()
            .take_while(|b| *b == b' ' || *b == b'\t')
            .count();
        let note = &rest_after_colon[space_after..];
        let has_space = ws_prefix_len > 0 || space_after > 0;
        let keyword = &text[..kw.len()];

        if colon || has_space {
            let mut chars = keyword.chars();
            let capitalized = if let Some(first) = chars.next() {
                let rest: String = chars.collect();
                format!(
                    "{}{}",
                    first.to_ascii_uppercase(),
                    rest.to_ascii_lowercase()
                )
            } else {
                String::new()
            };
            if !colon && has_space && !note.is_empty() && keyword == capitalized {
                return false;
            }
            return true;
        }
    }

    false
}

fn if_is_postfix_modifier(node: &ruby_prism::IfNode<'_>) -> bool {
    node.end_keyword_loc().is_none()
        && node.if_keyword_loc().is_some_and(|keyword| {
            node.statements().is_some_and(|statements| {
                keyword.start_offset() > statements.location().end_offset()
            })
        })
}

fn unless_is_postfix_modifier(node: &ruby_prism::UnlessNode<'_>) -> bool {
    node.end_keyword_loc().is_none()
        && node.statements().is_some_and(|statements| {
            node.keyword_loc().start_offset() > statements.location().end_offset()
        })
}

fn while_is_postfix_modifier(node: &ruby_prism::WhileNode<'_>) -> bool {
    node.do_keyword_loc().is_none()
        && node.statements().is_some_and(|statements| {
            node.keyword_loc().start_offset() > statements.location().end_offset()
        })
}

fn until_is_postfix_modifier(node: &ruby_prism::UntilNode<'_>) -> bool {
    node.do_keyword_loc().is_none()
        && node.statements().is_some_and(|statements| {
            node.keyword_loc().start_offset() > statements.location().end_offset()
        })
}

fn has_method_documentation_comment(source: &SourceFile, keyword_offset: usize) -> bool {
    let (node_line, _) = source.offset_to_line_col(keyword_offset);
    if node_line <= 1 {
        return false;
    }

    let lines: Vec<&[u8]> = source.lines().collect();
    let mut line_idx = node_line - 2;
    let mut found_doc_comment = false;
    let mut seen_any_comment = false;

    while let Some(line) = lines.get(line_idx) {
        let trimmed = trim_bytes(line);

        if trimmed.is_empty() {
            if found_doc_comment {
                break;
            }
            if seen_any_comment {
                seen_any_comment = false;
                if line_idx == 0 {
                    break;
                }
                line_idx -= 1;
                continue;
            }
            break;
        }

        if !trimmed.starts_with(b"#") {
            break;
        }

        seen_any_comment = true;
        let comment_text = std::str::from_utf8(trimmed).unwrap_or("");
        if !is_annotation_or_directive_case_insensitive(comment_text) {
            found_doc_comment = true;
        }

        if line_idx == 0 {
            break;
        }
        line_idx -= 1;
    }

    found_doc_comment
}

struct DocumentationMethodVisitor<'a> {
    cop: &'a DocumentationMethod,
    source: &'a SourceFile,
    require_for_non_public: bool,
    allowed_methods: Option<Vec<String>>,
    diagnostics: &'a mut Vec<Diagnostic>,
    pending_visibility: HashMap<usize, Option<MethodVisibility>>,
    wrapped_comment_depth: usize,
    inline_non_public_depth: usize,
}

impl DocumentationMethodVisitor<'_> {
    fn check_def(
        &mut self,
        def_node: &ruby_prism::DefNode<'_>,
        sibling_visibility: Option<MethodVisibility>,
    ) {
        let method_name = std::str::from_utf8(def_node.name().as_slice()).unwrap_or("");

        if method_name == "initialize" {
            return;
        }

        if let Some(ref allowed) = self.allowed_methods {
            if allowed.iter().any(|m| m == method_name) {
                return;
            }
        }

        let loc = def_node.location();
        let def_offset = loc.start_offset();
        let modifier = detect_inline_modifier(self.source, def_offset);

        if !self.require_for_non_public {
            if let Some((mod_bytes, _)) = modifier {
                if is_non_public_modifier(mod_bytes) {
                    return;
                }
            }
            if matches!(
                sibling_visibility,
                Some(MethodVisibility::Private | MethodVisibility::Protected)
            ) {
                return;
            }
            if self.inline_non_public_depth > 0 {
                return;
            }
        }

        // When the def has a non-modifier prefix on the same line (e.g.,
        // `__skip__ = def foo`, `class << x; def foo`, `Module.new { def foo`),
        // comments above the line belong to the outer expression, not the def.
        let prefix_steals_comments =
            modifier.is_none() && has_non_modifier_prefix(self.source, def_offset);

        if self.wrapped_comment_depth == 0
            && !prefix_steals_comments
            && has_method_documentation_comment(self.source, def_offset)
        {
            return;
        }

        let (line, column) = if let Some((_, indent)) = modifier {
            let (line, _) = self.source.offset_to_line_col(def_offset);
            (line, indent)
        } else {
            self.source.offset_to_line_col(def_offset)
        };

        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Missing method documentation comment.".to_string(),
        ));
    }
}

impl<'pr> Visit<'pr> for DocumentationMethodVisitor<'_> {
    fn visit_program_node(&mut self, node: &ruby_prism::ProgramNode<'pr>) {
        self.visit_statements_node(&node.statements());
    }

    fn visit_statements_node(&mut self, node: &ruby_prism::StatementsNode<'pr>) {
        let stmt_nodes: Vec<_> = node.body().iter().collect();
        let mut preceding_visibility = None;

        for (idx, stmt) in stmt_nodes.iter().enumerate() {
            if let Some(def_node) = stmt.as_def_node() {
                let method_name = std::str::from_utf8(def_node.name().as_slice()).unwrap_or("");
                let vis = retroactive_visibility_from_siblings(&stmt_nodes[idx + 1..], method_name)
                    .or(preceding_visibility);
                self.pending_visibility
                    .insert(def_node.location().start_offset(), vis);
            }

            // Register inner DefNodes of modifier calls (module_function/ruby2_keywords)
            // so they inherit the enclosing preceding visibility. RuboCop's on_def checks
            // `non_public?(parent)` for modifier_node? parents, which uses the parent
            // call's visibility in the statement list.
            if let Some(call) = stmt.as_call_node() {
                if call.receiver().is_none()
                    && matches!(
                        call.name().as_slice(),
                        b"module_function" | b"ruby2_keywords"
                    )
                {
                    if let Some(args) = call.arguments() {
                        for arg in args.arguments().iter() {
                            if let Some(inner_def) = arg.as_def_node() {
                                let method_name =
                                    std::str::from_utf8(inner_def.name().as_slice()).unwrap_or("");
                                let vis = retroactive_visibility_from_siblings(
                                    &stmt_nodes[idx + 1..],
                                    method_name,
                                )
                                .or(preceding_visibility);
                                self.pending_visibility
                                    .insert(inner_def.location().start_offset(), vis);
                            }
                        }
                    }
                }
            }

            self.visit(stmt);

            if let Some(vis) = standalone_visibility(stmt) {
                preceding_visibility = Some(vis);
            }
        }
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let visibility = self
            .pending_visibility
            .remove(&node.location().start_offset())
            .flatten();
        self.check_def(node, visibility);
        // Reset wrapped_comment_depth and inline_non_public_depth when entering
        // a def body. Comments inside a nested scope belong to the inner methods,
        // not to the outer wrapper. Without this reset, postfix modifiers like
        // `public_class_method def self.pry ... end` would leak wrapped_comment_depth
        // into all methods defined in the def body, and `private def foo ... end`
        // would leak inline_non_public_depth into nested singleton method defs.
        let saved_wrapped = self.wrapped_comment_depth;
        let saved_inline = self.inline_non_public_depth;
        self.wrapped_comment_depth = 0;
        self.inline_non_public_depth = 0;
        ruby_prism::visit_def_node(self, node);
        self.wrapped_comment_depth = saved_wrapped;
        self.inline_non_public_depth = saved_inline;
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        let saved_wrapped = self.wrapped_comment_depth;
        let saved_inline = self.inline_non_public_depth;
        self.wrapped_comment_depth = 0;
        self.inline_non_public_depth = 0;
        ruby_prism::visit_class_node(self, node);
        self.wrapped_comment_depth = saved_wrapped;
        self.inline_non_public_depth = saved_inline;
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        let saved_wrapped = self.wrapped_comment_depth;
        let saved_inline = self.inline_non_public_depth;
        self.wrapped_comment_depth = 0;
        self.inline_non_public_depth = 0;
        ruby_prism::visit_module_node(self, node);
        self.wrapped_comment_depth = saved_wrapped;
        self.inline_non_public_depth = saved_inline;
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        let saved_wrapped = self.wrapped_comment_depth;
        let saved_inline = self.inline_non_public_depth;
        self.wrapped_comment_depth = 0;
        self.inline_non_public_depth = 0;
        ruby_prism::visit_singleton_class_node(self, node);
        self.wrapped_comment_depth = saved_wrapped;
        self.inline_non_public_depth = saved_inline;
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            self.visit(&receiver);
        }

        if let Some(args) = node.arguments() {
            let steal_comments = call_wrapper_steals_comments(node);
            for arg in args.arguments().iter() {
                if steal_comments {
                    self.wrapped_comment_depth += 1;
                }
                if inline_non_public_call(node, &arg) {
                    self.inline_non_public_depth += 1;
                }

                self.visit(&arg);

                if inline_non_public_call(node, &arg) {
                    self.inline_non_public_depth -= 1;
                }
                if steal_comments {
                    self.wrapped_comment_depth -= 1;
                }
            }
        }

        if let Some(block) = node.block() {
            self.visit(&block);
        }
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        self.visit(&node.predicate());

        if let Some(statements) = node.statements() {
            if if_is_postfix_modifier(node) {
                self.wrapped_comment_depth += 1;
            }
            self.visit_statements_node(&statements);
            if if_is_postfix_modifier(node) {
                self.wrapped_comment_depth -= 1;
            }
        }

        if let Some(subsequent) = node.subsequent() {
            self.visit(&subsequent);
        }
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        self.visit(&node.predicate());

        if let Some(statements) = node.statements() {
            if unless_is_postfix_modifier(node) {
                self.wrapped_comment_depth += 1;
            }
            self.visit_statements_node(&statements);
            if unless_is_postfix_modifier(node) {
                self.wrapped_comment_depth -= 1;
            }
        }

        if let Some(else_clause) = node.else_clause() {
            self.visit(&else_clause.as_node());
        }
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        self.visit(&node.predicate());

        if let Some(statements) = node.statements() {
            if while_is_postfix_modifier(node) {
                self.wrapped_comment_depth += 1;
            }
            self.visit_statements_node(&statements);
            if while_is_postfix_modifier(node) {
                self.wrapped_comment_depth -= 1;
            }
        }
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        self.visit(&node.predicate());

        if let Some(statements) = node.statements() {
            if until_is_postfix_modifier(node) {
                self.wrapped_comment_depth += 1;
            }
            self.visit_statements_node(&statements);
            if until_is_postfix_modifier(node) {
                self.wrapped_comment_depth -= 1;
            }
        }
    }

    fn visit_rescue_modifier_node(&mut self, node: &ruby_prism::RescueModifierNode<'pr>) {
        self.wrapped_comment_depth += 1;
        self.visit(&node.expression());
        self.wrapped_comment_depth -= 1;
        self.visit(&node.rescue_expression());
    }
}

impl Cop for DocumentationMethod {
    fn name(&self) -> &'static str {
        "Style/DocumentationMethod"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["spec/**/*", "test/**/*"]
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
        let mut visitor = DocumentationMethodVisitor {
            cop: self,
            source,
            require_for_non_public: config.get_bool("RequireForNonPublicMethods", false),
            allowed_methods: config.get_string_array("AllowedMethods"),
            diagnostics,
            pending_visibility: HashMap::new(),
            wrapped_comment_depth: 0,
            inline_non_public_depth: 0,
        };
        visitor.visit(&parse_result.node());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(DocumentationMethod, "cops/style/documentation_method");
}
