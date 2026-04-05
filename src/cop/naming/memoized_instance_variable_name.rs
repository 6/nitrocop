use ruby_prism::Visit;

use crate::cop::shared::node_type::{
    CALL_NODE, DEF_NODE, IF_NODE, INSTANCE_VARIABLE_OR_WRITE_NODE, INSTANCE_VARIABLE_WRITE_NODE,
    STATEMENTS_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Naming/MemoizedInstanceVariableName — checks that memoized instance variables
/// match the method name.
///
/// ## Investigation (2026-03-08)
/// FN=27 in corpus. Root causes:
/// 1. Missing `define_method`/`define_singleton_method` support: RuboCop checks
///    memoization inside dynamically defined methods (`define_method(:foo) do @bar ||= ... end`).
///    Nitrocop only handled `DefNode`.
/// 2. Singleton methods (`def self.x`) were already handled since Prism represents them
///    as `DefNode` with `receiver().is_some()` — no code change needed for those.
///
/// Fix: Added `CallNode` handling in `check_node` for `define_method` and
/// `define_singleton_method` calls with blocks. Extracts method name from first
/// sym/str argument, then checks block body for `||=` or `defined?` memoization patterns.
///
/// ## Investigation (2026-03-08, second pass)
/// FN=23 in corpus. All FNs were `@ivar ||= expr` inside conditional branches
/// (if/else, unless modifier, case/when/else, ensure block, synchronize block, ternary).
///
/// Root cause: RuboCop uses `on_or_asgn` which fires for every `||=`, then checks
/// `body.children.last == node`. Parser AST's `children.last` traverses one level
/// into any node type: if → else branch, case → else clause, block → body,
/// ensure → ensure body. Nitrocop only checked direct body and last statement.
///
/// Fix: Added `get_last_child_or_write()` which replicates Parser's `children.last`
/// for single-statement method bodies. For multi-statement bodies, only the last
/// statement is checked directly (matching Parser's `begin.children.last` behavior).
///
/// Remaining FN=2: 1 is from Parser's structural equality (two identical `||=`
/// nodes in if/else branches compare as equal — a quirk we don't replicate),
/// 1 may be from config resolution differences.
///
/// ## Investigation (2026-03-09, regression fix)
/// Corpus oracle reported FP=3, FN=1 (previously 0 FP).
///
/// FP=3: All caused by `get_last_child_or_write()` being too aggressive:
///   1. (rails) `||=` in multi-statement else branch of `assign_controller` — in Parser AST,
///      multi-statement else maps to `begin` wrapper, `begin != or_asgn`, no match.
///   2. (rails) `||=` in multi-statement ensure block of `run_step_inline` — same issue.
///   3. (awspec) `||=` inside `define_method(:resource)` block inside `def self.aws_resource` —
///      RuboCop walks UP from `||=` and finds the `define_method` block first (not the enclosing
///      def), using method_name="resource" which matches `@resource`. Nitrocop walked DOWN from
///      `def aws_resource` and incorrectly traversed into the define_method block.
///
/// Fix: (a) `unwrap_single_stmt_to_or_write` and `get_last_child_or_write` now only unwrap
/// single-statement bodies (multi-statement = Parser's begin wrapper = no match).
/// (b) `get_last_child_or_write` skips `define_method`/`define_singleton_method` blocks
/// since those create their own method context handled by `check_dynamic_method`.
///
/// FN=1: (brakeman) `@attr_accessible ||= []` in if-branch of `set_attr_accessible`.
/// Root cause: Parser AST uses structural equality for `body.children.last == node`.
/// When if/else branches contain identical `||=` expressions (same ivar, same RHS),
/// Parser considers them equal, so RuboCop flags both. Nitrocop only found the else
/// branch via `get_last_child_or_write`. Fix: after finding a mismatch, scan the
/// method body for sibling `||=` nodes with the same ivar name and RHS source text.
///
/// ## Investigation (2026-03-20, extended corpus FP=3, FN=17)
/// FP=3: `||=` inside multi-statement block bodies (VCR.use_cassette with 2 stmts,
/// base.setup with 2 stmts). In Parser AST, multi-statement blocks are wrapped in
/// `begin` which is not `or_asgn`, so `children.last` doesn't match. Fix: changed
/// CallNode block handler in `get_last_child_or_write` to only unwrap single-statement
/// block bodies (via `unwrap_single_stmt_to_or_write_or_recurse`).
///
/// FN=17 from several patterns where `||=` is nested inside another node:
/// 1. `return @ivar ||= expr` (5 FN): ReturnNode wrapping ||=
/// 2. `yield @ivar ||= expr` (4 FN): YieldNode wrapping ||=
/// 3. `@ivar = @ivar ||= expr` (3 FN): InstanceVariableWriteNode wrapping ||=
/// 4. `@a ||= @b ||= expr` (3 FN): chained ||= where inner doesn't match
/// 5. `expr - @ivar ||= 0` (1 FN): CallNode (operator) with ||= as last arg
/// 6. `define_method "name" do return @ivar ||= expr end` (1 FN): return inside define_method
///
/// Root cause: RuboCop's `on_or_asgn` fires for EVERY `||=` in the method body, then
/// checks `body == node || body.children.last == node`. Parser's `children.last` returns
/// the rightmost child of any node type (return→arg, yield→arg, ivasgn→value, send→last_arg,
/// or_asgn→value). Nitrocop only checked specific node types in `get_last_child_or_write`.
///
/// Fix: (a) Added ReturnNode, YieldNode, InstanceVariableWriteNode, and CallNode-without-block
/// handlers to `get_last_child_or_write`. (b) Added `check_or_write_chain` to recursively
/// check nested `||=` in the value of an outer `||=` (chained pattern).
///
/// ## Corpus investigation (2026-03-23) — extended corpus
///
/// Extended corpus reported FP=2 (1 vendor, 1 genuine), FN=1.
/// FP=1 (genuine): `@automatic_inverse_of ||= ...` in `def inverse_name` at
/// samvera/active_fedora. The `||=` is deeply nested inside `options.fetch(:inverse_of) do
/// if @automatic_inverse_of == false ... else ... end`. RuboCop's `body.children.last`
/// returns the block body (an if-node), which is not equal to the `or_asgn` node, so
/// RuboCop does NOT flag this. Nitrocop correctly skips it via the CallNode block handler
/// in `get_last_child_or_write` which only matches single-statement block bodies directly.
/// Already covered in no_offense.rb fixture (`def inverse_name`).
///
/// FN=1: `@script ||= rest.first` in `def options` at brixen/poetics (`bin/poetics`).
/// Root cause: NOT a cop logic issue — the cop correctly flags `||=` as the last statement
/// of a multi-statement method body. The FN was caused by file discovery: `bin/poetics`
/// has shebang `#!/usr/bin/env rbx` (Rubinius), and `has_ruby_shebang()` in `fs.rs` only
/// checked for "ruby" in the shebang line. Fix: expanded `has_ruby_shebang()` to recognize
/// all interpreters from RuboCop's `AllCops.RubyInterpreters` (ruby, macruby, rake, jruby, rbx).
///
/// ## Variant style fix (2026-04-05) — FP=185 (required: 168, optional: 17)
///
/// Root cause: `check_or_write` and `check_defined_memoized` did not replicate
/// RuboCop's `variable_name_candidates` correctly for non-default styles.
///
/// RuboCop's `variable_name_candidates(method_name)`:
/// - `required`: `["_#{name}", name.start_with?('_') ? name : nil].compact`
/// - `optional`: `[name, "_#{name}", name.delete_prefix('_')]`
/// - `disallowed`: `[name, name.delete_prefix('_')]`
///
/// Nitrocop's `required` only accepted `_<base_name>`, missing the case where
/// the method itself starts with `_` (e.g., `def _foo` with `@_foo` was incorrectly
/// flagged as FP). Fix: also accept `base_name` when it starts with `_`.
///
/// Nitrocop's `optional` only accepted `base_name` and `_<base_name>`, missing
/// the `delete_prefix('_')` candidate (e.g., `def _foo` with `@foo` was incorrectly
/// flagged as FP). Fix: also accept `base_name.strip_prefix('_')`.
///
/// Both `check_or_write` and `check_defined_memoized` had the same bug and were
/// fixed identically.
pub struct MemoizedInstanceVariableName;

impl MemoizedInstanceVariableName {
    /// Check a `||=` node and also check nested `||=` in its value (chained pattern).
    /// RuboCop's `on_or_asgn` fires for EVERY `||=`, so `@a ||= @b ||= expr`
    /// flags both `@a` and `@b` independently. `body.children.last` for the outer
    /// `||=` returns the inner `||=`, which equals the inner node.
    fn check_or_write_chain(
        &self,
        source: &SourceFile,
        or_write: ruby_prism::InstanceVariableOrWriteNode<'_>,
        base_name: &str,
        method_name_str: &str,
        leading_underscore_style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        // Extract value before or_write is moved into check_or_write
        let value = or_write.value();
        diagnostics.extend(self.check_or_write(
            source,
            or_write,
            base_name,
            method_name_str,
            leading_underscore_style,
        ));
        // Check nested ||= in the value: @a ||= @b ||= expr
        if let Some(inner) = value.as_instance_variable_or_write_node() {
            self.check_or_write_chain(
                source,
                inner,
                base_name,
                method_name_str,
                leading_underscore_style,
                diagnostics,
            );
        }
    }

    fn check_or_write(
        &self,
        source: &SourceFile,
        or_write: ruby_prism::InstanceVariableOrWriteNode<'_>,
        base_name: &str,
        method_name_str: &str,
        leading_underscore_style: &str,
    ) -> Vec<Diagnostic> {
        let ivar_name = or_write.name().as_slice();
        let ivar_str = std::str::from_utf8(ivar_name).unwrap_or("");
        let ivar_base = ivar_str.strip_prefix('@').unwrap_or(ivar_str);

        let matches = match leading_underscore_style {
            "required" => {
                // RuboCop: variable_name_candidates = ["_#{method_name}",
                //   method_name.start_with?('_') ? method_name : nil].compact
                // So @_method_name is always valid, and @method_name is also
                // valid when the method itself starts with '_'.
                let with_underscore = format!("_{base_name}");
                ivar_base == with_underscore
                    || (base_name.starts_with('_') && ivar_base == base_name)
            }
            "optional" => {
                // RuboCop: variable_name_candidates = [method_name,
                //   "_#{method_name}", method_name.delete_prefix('_')]
                // So @method_name, @_method_name, and @method_name_without_leading_underscore
                // are all valid.
                let with_underscore = format!("_{base_name}");
                ivar_base == base_name
                    || ivar_base == with_underscore
                    || base_name
                        .strip_prefix('_')
                        .is_some_and(|stripped| ivar_base == stripped)
            }
            _ => {
                // "disallowed" (default): @method_name or @method_name_without_leading_underscore
                // RuboCop's variable_name_candidates returns [method_name, no_underscore]
                ivar_base == base_name
                    || base_name
                        .strip_prefix('_')
                        .is_some_and(|stripped| ivar_base == stripped)
            }
        };

        if !matches {
            let loc = or_write.name_loc();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            return vec![self.diagnostic(
                source,
                line,
                column,
                format!(
                    "Memoized variable `@{ivar_base}` does not match method name `{method_name_str}`."
                ),
            )];
        }

        Vec::new()
    }

    /// Handle `define_method(:name) do ... end` and `define_singleton_method(:name) do ... end`.
    /// Extracts the method name from the first sym/str argument, then checks the block body
    /// for memoization patterns (`||=` or `defined?`).
    fn check_dynamic_method(
        &self,
        source: &SourceFile,
        call_node: ruby_prism::CallNode<'_>,
        enforced_style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        // Extract method name from first argument (symbol or string)
        let args = match call_node.arguments() {
            Some(a) => a,
            None => return,
        };
        let args_list: Vec<_> = args.arguments().iter().collect();
        if args_list.is_empty() {
            return;
        }

        let name_bytes = if let Some(sym) = args_list[0].as_symbol_node() {
            sym.unescaped().to_vec()
        } else if let Some(s) = args_list[0].as_string_node() {
            s.unescaped().to_vec()
        } else {
            return;
        };

        let method_name_str = match std::str::from_utf8(&name_bytes) {
            Ok(s) => s,
            Err(_) => return,
        };

        // RuboCop skips initialize methods
        if matches!(
            method_name_str,
            "initialize" | "initialize_clone" | "initialize_copy" | "initialize_dup"
        ) {
            return;
        }

        let base_name = method_name_str.trim_end_matches(['?', '!', '=']);

        // Get the block body
        let block = match call_node.block() {
            Some(b) => b,
            None => return,
        };
        let block_node = match block.as_block_node() {
            Some(b) => b,
            None => return,
        };
        let body = match block_node.body() {
            Some(b) => b,
            None => return,
        };

        // Check for bare ||= as the entire body
        if let Some(or_write) = body.as_instance_variable_or_write_node() {
            diagnostics.extend(self.check_or_write(
                source,
                or_write,
                base_name,
                method_name_str,
                enforced_style,
            ));
            return;
        }

        let stmts = match body.as_statements_node() {
            Some(s) => s,
            None => return,
        };

        let body_nodes: Vec<_> = stmts.body().iter().collect();
        if body_nodes.is_empty() {
            return;
        }

        // Check last statement for ||=
        let last = &body_nodes[body_nodes.len() - 1];
        if let Some(or_write) = last.as_instance_variable_or_write_node() {
            diagnostics.extend(self.check_or_write(
                source,
                or_write,
                base_name,
                method_name_str,
                enforced_style,
            ));
            return;
        }

        // For single-statement block bodies, follow the "last child" chain
        if body_nodes.len() == 1 {
            if let Some(or_write) = get_last_child_or_write(last) {
                diagnostics.extend(self.check_or_write(
                    source,
                    or_write,
                    base_name,
                    method_name_str,
                    enforced_style,
                ));
                return;
            }
        }

        // Check defined? memoization pattern
        if body_nodes.len() >= 2 {
            if let Some(ivar_base) = extract_defined_memoized_ivar(&body_nodes) {
                diagnostics.extend(self.check_defined_memoized(
                    source,
                    &body_nodes,
                    &ivar_base,
                    base_name,
                    method_name_str,
                    enforced_style,
                ));
            }
        }
    }

    /// Check the `defined?` memoization pattern and emit offenses on each ivar reference.
    /// RuboCop emits one offense per ivar occurrence (defined? check, return, assignment).
    fn check_defined_memoized(
        &self,
        source: &SourceFile,
        body_nodes: &[ruby_prism::Node<'_>],
        ivar_base: &str,
        base_name: &str,
        method_name_str: &str,
        enforced_style: &str,
    ) -> Vec<Diagnostic> {
        let matches = match enforced_style {
            "required" => {
                let with_underscore = format!("_{base_name}");
                ivar_base == with_underscore
                    || (base_name.starts_with('_') && ivar_base == base_name)
            }
            "optional" => {
                let with_underscore = format!("_{base_name}");
                ivar_base == base_name
                    || ivar_base == with_underscore
                    || base_name
                        .strip_prefix('_')
                        .is_some_and(|stripped| ivar_base == stripped)
            }
            _ => {
                // "disallowed" (default): @method_name or @method_name_without_leading_underscore
                ivar_base == base_name
                    || base_name
                        .strip_prefix('_')
                        .is_some_and(|stripped| ivar_base == stripped)
            }
        };

        if matches {
            return Vec::new();
        }

        let suggested = match enforced_style {
            "required" => format!("_{base_name}"),
            _ => base_name.to_string(),
        };

        let msg = format!(
            "Memoized variable `@{ivar_base}` does not match method name `{method_name_str}`. Use `@{suggested}` instead."
        );

        // Collect all ivar locations from the defined? pattern:
        // 1. defined?(@ivar) — the ivar inside defined?
        // 2. return @ivar — the ivar in the return
        // 3. @ivar = ... — the assignment
        let mut diags = Vec::new();

        // The first node should be an if with defined?
        if let Some(if_node) = body_nodes[0].as_if_node() {
            // defined?(@ivar) — in the predicate
            if let Some(call) = if_node.predicate().as_call_node() {
                if call.name().as_slice() == b"defined?" {
                    if let Some(args) = call.arguments() {
                        for arg in args.arguments().iter() {
                            if arg.as_instance_variable_read_node().is_some() {
                                let loc = arg.location();
                                let (line, column) = source.offset_to_line_col(loc.start_offset());
                                diags.push(self.diagnostic(source, line, column, msg.clone()));
                            }
                        }
                    }
                }
            }
            // Also check if the predicate is a DefinedNode
            if let Some(defined) = if_node.predicate().as_defined_node() {
                let value = defined.value();
                if value.as_instance_variable_read_node().is_some() {
                    let loc = value.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diags.push(self.diagnostic(source, line, column, msg.clone()));
                }
            }

            // return @ivar — in the then/statements
            if let Some(stmts) = if_node.statements() {
                for stmt in stmts.body().iter() {
                    if let Some(ret) = stmt.as_return_node() {
                        if let Some(args) = ret.arguments() {
                            for arg in args.arguments().iter() {
                                if arg.as_instance_variable_read_node().is_some() {
                                    let loc = arg.location();
                                    let (line, column) =
                                        source.offset_to_line_col(loc.start_offset());
                                    diags.push(self.diagnostic(source, line, column, msg.clone()));
                                }
                            }
                        }
                    }
                }
            }
        }

        // The last node should be @ivar = ...
        let last = &body_nodes[body_nodes.len() - 1];
        if let Some(ivar_write) = last.as_instance_variable_write_node() {
            let loc = ivar_write.name_loc();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diags.push(self.diagnostic(source, line, column, msg));
        }

        diags
    }
}

/// Get the "last child" of a node in the Parser AST sense: one level of
/// `node.children.last`. This replicates how RuboCop's
/// `body.children.last == node` reaches through different node types.
///
/// In Parser AST, `children.last` on different node types:
/// - `if` → else branch (last of [cond, then, else])
/// - `case` → else clause (last of [expr, when..., else])
/// - `ensure` → ensure body (last of [body, ensure_body])
/// - `block` → block body (last of [send, args, body])
///
/// **Important**: In Parser AST, multi-statement sequences are wrapped in
/// `begin` nodes, and `begin_node != or_asgn_node`. So when the target
/// branch/clause has multiple statements, RuboCop's `children.last` returns
/// the `begin` wrapper — which never equals the `or_asgn` — and no offense
/// is raised. We replicate this by only unwrapping single-statement bodies.
///
/// Returns the "last child" Node, which may itself need StatementsNode
/// unwrapping to reach the final value.
fn get_last_child_or_write<'pr>(
    node: &ruby_prism::Node<'pr>,
) -> Option<ruby_prism::InstanceVariableOrWriteNode<'pr>> {
    // Direct match
    if let Some(or_write) = node.as_instance_variable_or_write_node() {
        return Some(or_write);
    }

    // StatementsNode → last statement (Prism wrapper, not a Parser operation).
    // In Parser AST, single-child sequences are bare nodes (no begin wrapper),
    // while multi-child sequences are wrapped in begin. Only traverse single-child
    // StatementsNodes to match Parser's behavior.
    if let Some(stmts) = node.as_statements_node() {
        let body: Vec<_> = stmts.body().iter().collect();
        if body.len() == 1 {
            return body.last().and_then(|n| get_last_child_or_write(n));
        }
        return None;
    }

    // Now apply ONE level of Parser's children.last:

    // IfNode → subsequent (else branch only, NOT elsif)
    // Parser: `if` children = [cond, then, else], last = else
    // For `if/else`, children.last is the else body.
    // For `if/elsif/else`, children.last is the elsif node (another IfNode).
    // RuboCop only checks one level: body.children.last == node.
    // So ||= inside elsif/else is NOT caught (2+ levels deep).
    if let Some(if_node) = node.as_if_node() {
        if let Some(subsequent) = if_node.subsequent() {
            if let Some(else_node) = subsequent.as_else_node() {
                return unwrap_single_stmt_to_or_write(else_node.statements());
            }
            // elsif case: don't recurse — RuboCop doesn't catch ||= through elsif chains
        }
        return None;
    }

    // ElseNode → statements (reached when body itself is an ElseNode, unlikely but safe)
    if let Some(else_node) = node.as_else_node() {
        return unwrap_single_stmt_to_or_write(else_node.statements());
    }

    // UnlessNode → statements (the unless body)
    // Parser: `unless cond body` = `if cond nil body`, last = body
    if let Some(unless_node) = node.as_unless_node() {
        return unwrap_single_stmt_to_or_write(unless_node.statements());
    }

    // CaseNode → else_clause
    // Parser: `case` children = [expr, when..., else], last = else
    if let Some(case_node) = node.as_case_node() {
        if let Some(else_clause) = case_node.else_clause() {
            return unwrap_single_stmt_to_or_write(else_clause.statements());
        }
        return None;
    }

    // BeginNode (rescue/ensure) → ensure_clause
    // Parser: `(ensure body ensure_body)` children.last = ensure_body
    if let Some(begin_node) = node.as_begin_node() {
        if let Some(ensure_clause) = begin_node.ensure_clause() {
            return unwrap_single_stmt_to_or_write(ensure_clause.statements());
        }
        return None;
    }

    // CallNode — two cases:
    // 1. With block → block body (Parser: `block` children = [send, args, body], last = body)
    //    Skip define_method/define_singleton_method blocks — those create their own
    //    method context and are handled separately by check_dynamic_method.
    //    IMPORTANT: only check if the block body IS the or_write directly, without
    //    recursing further. In Parser, block.children.last returns the block's body
    //    (e.g., an if-node), and RuboCop checks `body == or_asgn`. If the body is
    //    something else (like an if-node), it doesn't match — no recursion into the
    //    if/else branches. Recursing caused FP for patterns like:
    //    `options.fetch(:x) do if cond then nil else @ivar ||= val end end`
    // 2. Without block → last argument (Parser: `send` children = [recv, method, ...args], last = last arg)
    //    Handles patterns like `expr - @ivar ||= 0.0`
    if let Some(call_node) = node.as_call_node() {
        let method = call_node.name().as_slice();
        let method_str = std::str::from_utf8(method).unwrap_or("");
        if method_str == "define_method" || method_str == "define_singleton_method" {
            return None;
        }
        if let Some(block) = call_node.block() {
            if let Some(block_node) = block.as_block_node() {
                if let Some(body) = block_node.body() {
                    // Direct match: body IS the or_write
                    if let Some(or_write) = body.as_instance_variable_or_write_node() {
                        return Some(or_write);
                    }
                    // StatementsNode with single statement (Prism wrapper, no recursion)
                    return unwrap_single_stmt_to_or_write(body.as_statements_node());
                }
            }
            return None;
        }
        // No block: check last argument
        if let Some(args) = call_node.arguments() {
            let arg_list: Vec<_> = args.arguments().iter().collect();
            if let Some(last) = arg_list.last() {
                return last.as_instance_variable_or_write_node();
            }
        }
        return None;
    }

    // ParenthesesNode → body
    if let Some(parens) = node.as_parentheses_node() {
        if let Some(body) = parens.body() {
            return unwrap_single_stmt_to_or_write_or_recurse(&body);
        }
        return None;
    }

    // ReturnNode → last argument
    // Parser: `return` children = [arg], last = arg
    if let Some(ret_node) = node.as_return_node() {
        if let Some(args) = ret_node.arguments() {
            let arg_list: Vec<_> = args.arguments().iter().collect();
            if let Some(last) = arg_list.last() {
                return last.as_instance_variable_or_write_node();
            }
        }
        return None;
    }

    // YieldNode → last argument
    // Parser: `yield` children = [arg], last = arg
    if let Some(yield_node) = node.as_yield_node() {
        if let Some(args) = yield_node.arguments() {
            let arg_list: Vec<_> = args.arguments().iter().collect();
            if let Some(last) = arg_list.last() {
                return last.as_instance_variable_or_write_node();
            }
        }
        return None;
    }

    // InstanceVariableWriteNode → value
    // Parser: `ivasgn` children = [name, value], last = value
    // Handles `@service = @service ||= expr`
    if let Some(ivar_write) = node.as_instance_variable_write_node() {
        let value = ivar_write.value();
        return value.as_instance_variable_or_write_node();
    }

    None
}

/// Helper for block bodies: only unwrap single-statement bodies (matching Parser's
/// `begin` wrapper behavior), and recurse into the single statement if it's not
/// a direct `||=`.
fn unwrap_single_stmt_to_or_write_or_recurse<'pr>(
    node: &ruby_prism::Node<'pr>,
) -> Option<ruby_prism::InstanceVariableOrWriteNode<'pr>> {
    if let Some(or_write) = node.as_instance_variable_or_write_node() {
        return Some(or_write);
    }
    if let Some(stmts) = node.as_statements_node() {
        let body: Vec<_> = stmts.body().iter().collect();
        if body.len() != 1 {
            return None;
        }
        return get_last_child_or_write(&body[0]);
    }
    None
}

/// Helper: extract ||= from an optional StatementsNode with exactly ONE statement.
/// In Parser AST, multi-statement sequences are wrapped in `begin` nodes, and
/// `begin_node != or_asgn_node`, so RuboCop's `children.last == node` check fails
/// for multi-statement bodies. Only single-statement bodies are transparent (no begin wrapper).
fn unwrap_single_stmt_to_or_write<'pr>(
    stmts: Option<ruby_prism::StatementsNode<'pr>>,
) -> Option<ruby_prism::InstanceVariableOrWriteNode<'pr>> {
    let stmts = stmts?;
    let body: Vec<_> = stmts.body().iter().collect();
    if body.len() != 1 {
        return None;
    }
    body[0].as_instance_variable_or_write_node()
}

/// Extract the ivar name from a `defined?` memoization pattern.
/// Pattern: first statement is `return @ivar if defined?(@ivar)` (modifier if)
/// and last statement is `@ivar = expression`.
/// Returns the ivar name (e.g. "@token") if the pattern matches.
fn extract_defined_memoized_ivar(body_nodes: &[ruby_prism::Node<'_>]) -> Option<String> {
    if body_nodes.len() < 2 {
        return None;
    }

    // First statement: `return @ivar if defined?(@ivar)`
    // In Prism, this is an IfNode with:
    //   predicate: DefinedNode or CallNode(`defined?`)
    //   statements: ReturnNode with ivar argument
    let first = &body_nodes[0];
    let if_node = first.as_if_node()?;

    // Check predicate is `defined?(@ivar)`
    // Note: Prism's name().as_slice() for ivar nodes includes the '@' prefix.
    // We strip it here to get the base name for comparison.
    let defined_ivar_base = if let Some(defined) = if_node.predicate().as_defined_node() {
        // DefinedNode has a .value() that returns the argument
        let value = defined.value();
        let ivar = value.as_instance_variable_read_node()?;
        let full = std::str::from_utf8(ivar.name().as_slice()).ok()?;
        full.strip_prefix('@').unwrap_or(full).to_string()
    } else if let Some(call) = if_node.predicate().as_call_node() {
        // Fallback: CallNode with name `defined?`
        if call.name().as_slice() != b"defined?" {
            return None;
        }
        let args = call.arguments()?;
        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.len() != 1 {
            return None;
        }
        let ivar = arg_list[0].as_instance_variable_read_node()?;
        let full = std::str::from_utf8(ivar.name().as_slice()).ok()?;
        full.strip_prefix('@').unwrap_or(full).to_string()
    } else {
        return None;
    };

    // Check then-body has `return @ivar`
    let stmts = if_node.statements()?;
    let stmt_nodes: Vec<_> = stmts.body().iter().collect();
    if stmt_nodes.len() != 1 {
        return None;
    }
    let ret = stmt_nodes[0].as_return_node()?;
    let ret_args = ret.arguments()?;
    let ret_arg_list: Vec<_> = ret_args.arguments().iter().collect();
    if ret_arg_list.len() != 1 {
        return None;
    }
    let ret_ivar = ret_arg_list[0].as_instance_variable_read_node()?;
    let ret_full = std::str::from_utf8(ret_ivar.name().as_slice()).ok()?;
    let ret_ivar_base = ret_full.strip_prefix('@').unwrap_or(ret_full);
    if ret_ivar_base != defined_ivar_base {
        return None;
    }

    // Last statement: `@ivar = expression`
    let last = &body_nodes[body_nodes.len() - 1];
    let ivar_write = last.as_instance_variable_write_node()?;
    let write_full = std::str::from_utf8(ivar_write.name().as_slice()).ok()?;
    let write_base = write_full.strip_prefix('@').unwrap_or(write_full);
    if write_base != defined_ivar_base {
        return None;
    }

    // Return the base name (without '@')
    Some(defined_ivar_base)
}

/// Visitor that collects all `@ivar ||= value` nodes matching a target ivar name
/// and value source text. Used to replicate Parser's structural equality: when
/// `body.children.last` finds an `or_asgn`, Parser's `==` treats any other `or_asgn`
/// with identical structure as matching, even if at a different position (e.g., the
/// same `||=` in both if and else branches).
struct OrWriteCollector<'a> {
    target_ivar_name: &'a [u8],
    target_value_src: &'a [u8],
    /// Offset of the reference node (already flagged — skip it)
    exclude_offset: usize,
    /// Offsets of matching sibling nodes
    sibling_offsets: Vec<usize>,
}

impl<'pr> Visit<'pr> for OrWriteCollector<'_> {
    fn visit_instance_variable_or_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOrWriteNode<'pr>,
    ) {
        if node.location().start_offset() == self.exclude_offset {
            return;
        }
        if node.name().as_slice() == self.target_ivar_name
            && node.value().location().as_slice() == self.target_value_src
        {
            self.sibling_offsets.push(node.name_loc().start_offset());
        }
    }
}

impl Cop for MemoizedInstanceVariableName {
    fn name(&self) -> &'static str {
        "Naming/MemoizedInstanceVariableName"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CALL_NODE,
            DEF_NODE,
            IF_NODE,
            INSTANCE_VARIABLE_OR_WRITE_NODE,
            INSTANCE_VARIABLE_WRITE_NODE,
            STATEMENTS_NODE,
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
        let enforced_style = config.get_str("EnforcedStyleForLeadingUnderscores", "disallowed");

        // Handle define_method/define_singleton_method calls with blocks
        if let Some(call_node) = node.as_call_node() {
            let method = call_node.name().as_slice();
            let method_str = std::str::from_utf8(method).unwrap_or("");
            if method_str == "define_method" || method_str == "define_singleton_method" {
                self.check_dynamic_method(source, call_node, enforced_style, diagnostics);
            }
            return;
        }

        let def_node = match node.as_def_node() {
            Some(d) => d,
            None => return,
        };

        let method_name = def_node.name().as_slice();
        let method_name_str = std::str::from_utf8(method_name).unwrap_or("");

        // RuboCop skips initialize methods — `||=` there is default initialization, not memoization
        if matches!(
            method_name_str,
            "initialize" | "initialize_clone" | "initialize_copy" | "initialize_dup"
        ) {
            return;
        }

        // Strip trailing ?, !, or = from method name for matching
        // RuboCop does method_name.to_s.delete('!?=')
        let base_name = method_name_str.trim_end_matches(['?', '!', '=']);

        let body = match def_node.body() {
            Some(b) => b,
            None => return,
        };

        // Look for @var ||= pattern — only when it's the entire body or the
        // "last child" in the RuboCop sense. RuboCop checks body.children.last == node,
        // which traverses one level into any node type (if → else, case → else,
        // ensure → ensure body, block → body, etc). We replicate this with
        // get_last_child_or_write which recursively follows the chain.

        // Body could be a bare InstanceVariableOrWriteNode (single statement)
        if let Some(or_write) = body.as_instance_variable_or_write_node() {
            self.check_or_write_chain(
                source,
                or_write,
                base_name,
                method_name_str,
                enforced_style,
                diagnostics,
            );
            return;
        }

        // Body could be a BeginNode (rescue/ensure) — check via recursive last child
        if body.as_begin_node().is_some() {
            if let Some(or_write) = get_last_child_or_write(&body) {
                self.check_or_write_chain(
                    source,
                    or_write,
                    base_name,
                    method_name_str,
                    enforced_style,
                    diagnostics,
                );
            }
            return;
        }

        let stmts = match body.as_statements_node() {
            Some(s) => s,
            None => return,
        };

        let body_nodes: Vec<_> = stmts.body().iter().collect();
        if body_nodes.is_empty() {
            return;
        }

        // Check the last statement directly for ||=
        let last = &body_nodes[body_nodes.len() - 1];
        if let Some(or_write) = last.as_instance_variable_or_write_node() {
            self.check_or_write_chain(
                source,
                or_write,
                base_name,
                method_name_str,
                enforced_style,
                diagnostics,
            );
            return;
        }

        // For SINGLE-STATEMENT bodies: follow the "last child" chain through
        // the statement to find ||= inside if/else, unless, case, block, etc.
        //
        // In Parser AST, single-statement bodies have no `begin` wrapper,
        // so `body.children.last` digs into the statement's children.
        // For multi-statement bodies, `body.children.last` just returns the
        // last statement — no further traversal. We only apply the
        // "children.last" traversal for single-statement bodies.
        if body_nodes.len() == 1 {
            if let Some(or_write) = get_last_child_or_write(last) {
                // Extract values before or_write is moved into check_or_write_chain
                let ivar_name_bytes = or_write.name().as_slice().to_vec();
                let value_src = or_write.value().location().as_slice().to_vec();
                let ref_offset = or_write.location().start_offset();
                let count_before = diagnostics.len();
                self.check_or_write_chain(
                    source,
                    or_write,
                    base_name,
                    method_name_str,
                    enforced_style,
                    diagnostics,
                );
                if diagnostics.len() > count_before {
                    // Parser structural equality: scan body for sibling ||= nodes
                    // with the same ivar name and value source. RuboCop's
                    // `body.children.last == node` uses Parser's s-expression
                    // equality, so identical ||= in different branches (e.g.,
                    // if/else) both match.
                    let ivar_base = std::str::from_utf8(&ivar_name_bytes)
                        .unwrap_or("")
                        .strip_prefix('@')
                        .unwrap_or("");
                    let mut collector = OrWriteCollector {
                        target_ivar_name: &ivar_name_bytes,
                        target_value_src: &value_src,
                        exclude_offset: ref_offset,
                        sibling_offsets: Vec::new(),
                    };
                    collector.visit(last);
                    for offset in collector.sibling_offsets {
                        let (line, column) = source.offset_to_line_col(offset);
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            format!(
                                "Memoized variable `@{ivar_base}` does not match method name `{method_name_str}`.",
                            ),
                        ));
                    }
                }
                return;
            }
        }

        // Also check the `defined?` memoization pattern:
        //   return @ivar if defined?(@ivar)
        //   @ivar = expression
        // The first statement must be `if defined?(@ivar) then return @ivar end`
        // and the last statement must be `@ivar = expression`.
        if body_nodes.len() >= 2 {
            if let Some(ivar_base) = extract_defined_memoized_ivar(&body_nodes) {
                diagnostics.extend(self.check_defined_memoized(
                    source,
                    &body_nodes,
                    &ivar_base,
                    base_name,
                    method_name_str,
                    enforced_style,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        MemoizedInstanceVariableName,
        "cops/naming/memoized_instance_variable_name"
    );

    #[test]
    fn required_style_allows_leading_underscore() {
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForLeadingUnderscores".to_string(),
                serde_yml::Value::String("required".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def js_modules\n  @_js_modules ||= compute_modules\nend\n";
        assert_cop_no_offenses_full_with_config(&MemoizedInstanceVariableName, source, config);
    }

    #[test]
    fn optional_style_allows_both_forms() {
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForLeadingUnderscores".to_string(),
                serde_yml::Value::String("optional".to_string()),
            )]),
            ..CopConfig::default()
        };
        // Both forms should be accepted
        let source = b"def js_modules\n  @_js_modules ||= compute_modules\nend\n";
        assert_cop_no_offenses_full_with_config(
            &MemoizedInstanceVariableName,
            source,
            config.clone(),
        );
        let source2 = b"def js_modules\n  @js_modules ||= compute_modules\nend\n";
        assert_cop_no_offenses_full_with_config(&MemoizedInstanceVariableName, source2, config);
    }

    #[test]
    fn brakeman_set_attr_accessible_pattern() {
        // Corpus FN=1: method `set_attr_accessible` with `@attr_accessible ||= []`
        // in both if and else branches. RuboCop flags BOTH due to Parser structural
        // equality: the else-branch ||= is body.children.last, and the if-branch ||=
        // is structurally identical so `body.children.last == node` is true for both.
        let source = b"def set_attr_accessible exp = nil\n  if exp\n    args = []\n\n    exp.each_arg do |e|\n      if node_type? e, :lit\n        args << e.value\n      elsif hash? e\n        @role_accessible.concat args\n      end\n    end\n\n    @attr_accessible ||= []\n    @attr_accessible.concat args\n  else\n    @attr_accessible ||= []\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&MemoizedInstanceVariableName, source);
        assert_eq!(
            diags.len(),
            2,
            "Should flag both @attr_accessible ||= [] instances (structural equality)"
        );
    }

    #[test]
    fn multi_stmt_body_last_is_or_write_mismatch() {
        // Corpus FN=1: brixen/poetics — method `options` with multiple statements,
        // last statement is `@script ||= rest.first`. RuboCop flags it because
        // `body.children.last` (== the last statement of the begin wrapper) is the ||= node.
        let source = b"def options\n  options.doc \"\"\n  rest = options.parse(argv)\n  @script ||= rest.first\nend\n";
        let diags = crate::testutil::run_cop_full(&MemoizedInstanceVariableName, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag @script ||= in multi-statement body where last stmt is ||= with mismatched name"
        );
    }

    #[test]
    fn multi_stmt_body_with_blocks_last_is_or_write_mismatch() {
        // Corpus FN=1: brixen/poetics — method `options` has block calls
        // (`options.on ... do ... end`) before `@script ||= rest.first` as
        // the last statement. The cop logic is correct; the actual FN was
        // caused by file discovery not recognizing `#!/usr/bin/env rbx`
        // (Rubinius) shebangs. Fixed in fs.rs has_ruby_shebang().
        let source = b"def options(argv=ARGV)\n  options = Rubinius::Options.new \"Usage\", 20\n\n  options.on \"-e\", \"CODE\", \"Execute CODE\" do |e|\n    @evals << e\n  end\n\n  options.on \"-h\", \"--help\", \"Display this help\" do\n    puts options\n    exit 0\n  end\n\n  options.doc \"\"\n\n  rest = options.parse(argv)\n  @script ||= rest.first\nend\n";
        let diags = crate::testutil::run_cop_full(&MemoizedInstanceVariableName, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag @script ||= in method with block calls before it"
        );
    }

    #[test]
    fn required_style_flags_missing_underscore() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForLeadingUnderscores".to_string(),
                serde_yml::Value::String("required".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def js_modules\n  @js_modules ||= compute_modules\nend\n";
        let diags = run_cop_full_with_config(&MemoizedInstanceVariableName, source, config);
        assert!(
            !diags.is_empty(),
            "required style should flag @js_modules (missing underscore)"
        );
    }

    #[test]
    fn required_style_accepts_underscore_method_with_matching_ivar() {
        // RuboCop: variable_name_candidates("_foo") in required mode =
        // ["__foo", "_foo"]. So @_foo matches method _foo.
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForLeadingUnderscores".to_string(),
                serde_yml::Value::String("required".to_string()),
            )]),
            ..CopConfig::default()
        };
        // def _foo with @_foo should be accepted in required style
        let source = b"def _foo\n  @_foo ||= compute\nend\n";
        assert_cop_no_offenses_full_with_config(&MemoizedInstanceVariableName, source, config);
    }

    #[test]
    fn required_style_accepts_underscore_method_with_double_underscore_ivar() {
        // RuboCop: variable_name_candidates("_foo") in required mode =
        // ["__foo", "_foo"]. So @__foo also matches method _foo.
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForLeadingUnderscores".to_string(),
                serde_yml::Value::String("required".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def _foo\n  @__foo ||= compute\nend\n";
        assert_cop_no_offenses_full_with_config(&MemoizedInstanceVariableName, source, config);
    }

    #[test]
    fn optional_style_accepts_no_underscore_form() {
        // RuboCop: variable_name_candidates("_foo") in optional mode =
        // ["_foo", "__foo", "foo"]. So @foo matches method _foo.
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForLeadingUnderscores".to_string(),
                serde_yml::Value::String("optional".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def _foo\n  @foo ||= compute\nend\n";
        assert_cop_no_offenses_full_with_config(&MemoizedInstanceVariableName, source, config);
    }

    #[test]
    fn required_style_defined_pattern_accepts_underscore_method() {
        // defined? pattern should also respect required style matching
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForLeadingUnderscores".to_string(),
                serde_yml::Value::String("required".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def _foo\n  return @_foo if defined?(@_foo)\n  @_foo = compute\nend\n";
        assert_cop_no_offenses_full_with_config(&MemoizedInstanceVariableName, source, config);
    }
}
