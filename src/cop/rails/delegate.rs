use crate::cop::shared::access_modifier_predicates;
use crate::cop::shared::node_type::{
    CALL_NODE, CLASS_VARIABLE_READ_NODE, CONSTANT_PATH_NODE, CONSTANT_READ_NODE, DEF_NODE,
    GLOBAL_VARIABLE_READ_NODE, INSTANCE_VARIABLE_READ_NODE, LOCAL_VARIABLE_READ_NODE,
    REQUIRED_PARAMETER_NODE, SELF_NODE, STATEMENTS_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Rails/Delegate cop detects method definitions that simply delegate to another object,
/// suggesting the use of Rails' `delegate` macro instead.
///
/// ## Investigation findings (2026-03-10)
///
/// **FP root causes (49 FP):**
/// - Missing `module_function` check: RuboCop skips methods in modules that declare
///   `module_function`. Our cop was flagging these methods incorrectly.
/// - Missing `private :method_name` handling: The `is_private_or_protected` utility
///   only checked for standalone `private` keyword and inline `private def`, not
///   the `private :method_name` form that makes a specific method private after definition.
///
/// **FN root causes (136 FN):**
/// - Missing prefixed delegation detection: When `EnforceForPrefixed: true` (default),
///   `def bar_foo; bar.foo; end` should be flagged as a delegation that can use
///   `delegate :foo, to: :bar, prefix: true`. Our cop only matched exact method names.
///
/// **Fixes applied:**
/// - Added `module_function` detection via line scanning in enclosing scope
/// - Added `private :method_name` form detection
/// - Added prefixed delegation matching when `EnforceForPrefixed: true`
/// - Extended prefix skip (for `EnforceForPrefixed: false`) to all receiver types
///
/// ## Investigation (2026-03-14): FP=20
///
/// **FP root cause**: `is_in_module_function_scope` only scanned BACKWARDS from the def
/// for `module_function`. Patterns like `end; module_function :adapters` (inline after
/// the def's `end`) and `module_function :method_name` declared later in the module body
/// were missed. RuboCop's `module_function_declared?` walks ALL descendants of the
/// ancestor module — both before and after the def.
///
/// Fix: Added forward scan from the def line that checks if any subsequent line in the
/// same scope contains `module_function` (including `module_function :name` symbol forms).
///
/// ## Investigation (2026-03-15): FP=12
///
/// **FP root cause 1**: `is_in_module_function_scope` backward scan stopped at `class `
/// boundaries. When a class was nested inside a module that declared `module_function`
/// (e.g., `module Open4; module_function :open4; class SpawnError; def exitstatus`),
/// the backward scan would hit `class SpawnError` and break before reaching
/// `module_function :open4`. RuboCop's `module_function_declared?` checks ALL ancestors.
///
/// Fix: Changed backward scan to expand the search depth when crossing class boundaries,
/// so `module_function` at the outer module level is still found.
///
/// **FP root cause 2**: Endless methods (`def foo() = expr`) were flagged. RuboCop's
/// NodePattern matches `(def _method_name _args (send ...))` which in Parser gem doesn't
/// match endless defs. In Prism, endless defs have `equal_loc().is_some()`.
///
/// Fix: Skip defs with `equal_loc().is_some()` (endless method syntax).
///
/// **FP root cause 3**: `is_private_or_protected` (in util.rs) didn't match `private `
/// with a trailing space on its own line. The check compared against exact bytes
/// `b"private"` and specific continuations (`\n`, `\r`, ` #`) but not trailing spaces.
///
/// Fix: Added `starts_with(b"private ")` match that validates the remainder is
/// only whitespace (to avoid matching `private :method_name` or `private def foo`).
///
/// ## Investigation (2026-03-15): FP=1, FN=102
///
/// **FN root cause 1 (~majority)**: Endless methods (`def foo = bar.foo`) were incorrectly
/// skipped. The previous investigation assumed RuboCop's `(def ...)` NodePattern didn't
/// match endless defs, but corpus data proves RuboCop DOES flag them. In Prism, endless
/// methods have the body as a direct child (not wrapped in StatementsNode).
///
/// Fix: Removed the `equal_loc().is_some()` early return. Added fallback path that
/// handles the body as a direct CallNode when it's not a StatementsNode.
///
/// **FN root cause 2**: Prefixed delegation via `self.class` receiver
/// (e.g., `def class_name; self.class.name; end`) was not detected. `get_receiver_name`
/// only returned names for receiverless calls, but `self.class` has a receiver (`self`).
/// RuboCop's `determine_prefixed_method_receiver_name` returns `receiver.method_name`
/// for send nodes, which would be `"class"` for `self.class`.
///
/// Fix: Added handling in `get_receiver_name` for call nodes with a `self` receiver,
/// returning the method name (e.g., `"class"` for `self.class`).
///
/// **FP (1, antiwork/gumroad)**: `def to_stripejs_customer_id; to_stripejs_customer.id; end`
/// flagged by nitrocop but not RuboCop. Likely a private/module_function scope issue
/// in the full file that our line-based scanning doesn't detect. Cannot verify without
/// corpus file access.
///
/// **Remaining FNs**: 102 FNs in corpus, mostly in files not locally available.
/// Many are likely the endless method and self.class patterns now fixed. Others may
/// involve scope/visibility patterns not yet detected by line-based scanning.
///
/// ## Investigation (2026-03-15): FP=2, FN=28
///
/// **FN root cause 1**: `is_private_symbol_arg` was too broad — it matched
/// `private :method_name, :other` (multi-symbol calls). RuboCop's `VisibilityHelp`
/// pattern `(send nil? VISIBILITY_SCOPES (sym %method_name))` only matches single-symbol
/// `private :method_name`. Multi-symbol calls like `private :[]=, :set_element` do NOT
/// make the method private for delegate purposes.
///
/// Fix: Added comma check in `is_private_symbol_arg` to reject multi-symbol calls.
///
/// **FN root cause 2**: `is_in_module_function_scope` forward scan was too broad:
/// (a) matched `module_function` in comments (e.g., `# module_function...`),
/// (b) matched `module_function` in nested scopes at deeper indentation (e.g.,
/// `namespace :parallel do; module X; module_function; end; end`).
///
/// Fix: Added comment filtering (strip `#`-prefixed content) and indent check
/// (`indent <= def_col`) in the forward scan to restrict matches to the same scope.
///
/// **FP 1 (antiwork/gumroad)**: `def to_stripejs_customer_id; to_stripejs_customer.id; end`
/// correctly matched as prefixed delegation but RuboCop doesn't flag it. Without corpus
/// file access, cannot determine visibility context (likely private block earlier in file).
///
/// **FP 2 (palkan/anyway_config)**: `def clear() = value.clear` — endless method
/// delegation. RuboCop doesn't flag it. Without corpus access, cannot determine visibility
/// context (likely private block earlier in file).
///
/// ## Investigation (2026-03-16): FP=2, FN=24
///
/// **FN root cause**: `is_in_module_function_scope` forward scan used substring matching
/// (`windows().any()`) to detect `module_function`. This falsely matched identifiers
/// containing `module_function` as a substring, e.g., `register_module_function`,
/// `module_function?`, `make_module_function`. This was the primary FN source —
/// particularly in yard (10 FNs), where `lib/yard/handlers/base.rb` has delegation
/// methods like `def owner; parser.owner end` followed later by method
/// `def register_module_function(object)` which contains the substring.
///
/// Fix: Replaced `windows()` substring matching with `has_module_function_token()`
/// that checks word boundaries — `module_function` must be preceded and followed by
/// non-identifier characters (not alphanumeric, `_`, `?`, `!`).
///
/// **FP 1 & 2**: Both FPs remain — they are caused by visibility context (private
/// block earlier in the file) that our line-based scanning doesn't detect. The methods
/// ARE valid delegation patterns that RuboCop flags when public, confirmed via testing.
///
/// ## Investigation (2026-03-18): FP=2, FN=14
///
/// **FP root causes (2 FP — gumroad and anyway_config)**:
/// Both cases have `module_function` declared in an OUTER ancestor module, AFTER a nested
/// class/module definition in that outer module. The forward scan in `is_in_module_function_scope`
/// broke as soon as it encountered a `class`/`module` at `indent < def_col`, stopping before
/// it could reach the `module_function` in the outer scope.
///
/// Example (gumroad): `def to_stripejs_customer_id` inside `module ExtensionMethods` (indent 4).
/// After `ExtensionMethods` ends, `class StripeJs` appears at indent 2 (the outer scope).
/// The scan stopped at `class StripeJs`, never reaching `module_function` at indent 2 in
/// `module StripePaymentMethodHelper`. RuboCop's `module_function_declared?` checks ALL
/// ancestors, so it finds it.
///
/// Fix: Changed forward scan to track `sibling_scope_depth`. When `class`/`module` at
/// `indent < def_col` is encountered, increment depth (skip its body). When the matching
/// `end` is seen, decrement. Only check `module_function` when `sibling_scope_depth == 0`.
/// This allows scanning past sibling class/module bodies to find `module_function` in the
/// outer ancestor scope.
///
/// **FN root causes (14 FN — mongomapper, rouge, rails, coderay, etc.)**:
/// All FNs involve delegations via `def foo; self.class.foo; end` inside a module that
/// has a sibling `module ClassMethods` (or `class << self`) at the same indent level.
/// The sibling has `private` declared inside it at the same indent. The forward scan in
/// `is_private_or_protected` (which scans top-to-bottom) set `in_private = true` when
/// it encountered `private` inside the sibling, and never reset it when the sibling's
/// `end` was reached (because `end` at `indent == def_col` did not reset in_private).
///
/// Example (mongomapper): `module ClassMethods` at indent 6, with `private` at indent 6
/// inside it. After `end` of ClassMethods, `def associations` at indent 6 was incorrectly
/// considered private.
///
/// Fix: Added `peer_scope_depth` tracking in `is_private_or_protected`. When `class`/`module`
/// at `indent == def_col` is encountered, increment depth (entering a peer scope). When `end`
/// at `indent == def_col` decrements it to 0 (exiting peer scope), `in_private` updates are
/// skipped while inside the peer scope. This prevents `private` from inside sibling
/// class/modules from bleeding into instance methods at the same level.
///
/// ## Investigation (2026-03-18): FP=1, FN=2
///
/// **FP (rubocop, line 88)**: Already fixed by prior `is_private_or_protected` improvements.
/// `private` at same indent as `def` in deeply nested class correctly detected.
///
/// **FN (aruba, line 149)**: Already fixed by prior `peer_scope_depth` improvements.
/// `def mode; @announcer.mode; end` after `public` keyword correctly detected.
///
/// **FN (asciidoctor, line 66)**: `def now; ::Time.now; end` inside `if/else` block after
/// `private`. RuboCop's `node_visibility` uses AST sibling checks — a `def` inside an
/// `if/else` body is NOT a sibling of `private` in the class body, so RuboCop considers
/// it public. Our line-based `is_private_or_protected` incorrectly set `in_private = true`
/// because `private` at indent 4 <= def_col 6.
///
/// Fix: Added `is_inside_conditional_block()` check in the delegate cop. After
/// `is_private_or_protected` returns true, scan backwards from the def for block-opening
/// keywords (if/unless/case/else/elsif/while/etc.) at lower indent. If found, the def
/// is inside a conditional block and `private` doesn't apply per RuboCop's AST semantics.
///
/// ## Investigation (2026-03-19): FP=8, FN=0
///
/// **FP root cause (all 8)**: `is_inside_conditional_block()` backward scan didn't stop
/// at `end` keywords at indent < def_col. It scanned through sibling method/block bodies
/// and falsely matched conditional keywords (rescue/ensure/elsif/if) from INSIDE those
/// other methods. Examples:
/// - rails/rails: `ensure` at indent 2 inside a test method → falsely marked `def mkdir`
///   (at indent 4 after `private`) as inside a conditional block.
/// - ruby/debug: `elsif` at indent 6 inside `setup_sigdump` → falsely marked `private def
///   config` (at indent 12) as inside a conditional.
/// - antiwork/gumroad: `rescue` at indent 2 inside other methods → falsely marked
///   `def paypal_api` (at indent 4 after `private`) as inside a conditional.
///
/// Fix: Added `end` at indent < def_col as a stop condition in the backward scan of
/// `is_inside_conditional_block()`. When scanning backwards, an `end` at lower indent
/// closes a sibling scope — conditional keywords beyond it are in a different scope and
/// should not affect the current def.
///
/// ## Reverted fix attempt (2026-03-23, commit 0956d7b0)
///
/// Attempted to fix FP on parameter receivers and FN inside else blocks.
/// Introduced FP=1 on standard corpus; reverted in 1bf1bea3.
///
/// **FP=1 (is_inside_conditional_block overrides private in same branch):**
/// `def connection` (indent 6) inside an `else` branch (indent 4) preceded by
/// `private` (indent 6) in the same else branch. `is_private_or_protected`
/// returns true, but `is_inside_conditional_block` also returns true (finds
/// `else` at lower indent). The skip logic `is_private && !is_inside_conditional`
/// evaluates to false, so the private method gets flagged. But `private` is in
/// the SAME conditional branch as the def — it should still apply. Fix: when
/// `is_inside_conditional_block` is true, check whether `private` appears AFTER
/// the enclosing conditional keyword and BEFORE the def (same nesting level),
/// which means private still applies within that branch.
///
/// ## Investigation (2026-03-26): representative FN fixtures pass, corpus FN remain
///
/// Added the current representative corpus FN snippets to the fixture:
/// `@attribute_manager.add_word_pair(start, stop, name)`, `@attrs[n]`,
/// `@items << item`, `@parts.empty?`, `@parts.length`, and `def pop = frames.pop`.
/// The cop matches those cases in the unit fixture, so the remaining corpus FN=78
/// are not caused by the local delegation matcher.
///
/// Direct corpus reproduction showed a split between explicit-file and repo-root runs:
/// - Passing a missed file explicitly to nitrocop with the corpus config reports
///   the expected offenses (for example `rdoc/markup.rb` lines 594/601/614 and
///   `rdoc/markup/list.rb` lines 28/54/61).
/// - Running the same repo through `bench/corpus.run_nitrocop(..., cop='Rails/Delegate')`
///   omits those files entirely from the offense set.
///
/// The same pattern reproduced for `amuta__kumi__790c2e0`:
/// `lib/kumi/core/analyzer/passes/lir/lower_pass.rb:48` is flagged when passed
/// explicitly, but disappears in the repo-root corpus run.
///
/// Conclusion: the remaining corpus FN are dominated by whole-repo execution
/// dropping or suppressing eligible files before this cop runs. The likely fix
/// is outside this cop (file discovery / global exclude / repo-root config
/// handling in `fs.rs`, `linter.rs`, or `config/mod.rs`). No narrow matcher
/// change here fixes the corpus gap without papering over the real issue.
///
/// ## Investigation (2026-03-30): FP=2, FN=2
///
/// - FP: parameter/local-variable receivers like `def delete(x); x.delete(x); end`
///   are not matched by RuboCop's node pattern, so `LocalVariableReadNode`
///   receivers and prefixed names derived from them must be ignored.
/// - FP: Prism reports `def !@` as `name == "!"` while the source spelling is
///   `!@`. Matching on `name_loc()` keeps unary `!@` from falsely looking like a
///   delegation to unary `!`.
/// - FN: outer `private` leaked into defs nested inside non-scope bodies
///   (`Struct.new do ... end`, `if/else`, etc.). RuboCop only applies outer
///   visibility to sibling defs in the same scope, so nested bodies now get an
///   AST-aware override unless they declare visibility inline or within that same
///   nested body.
/// - FP: heredoc help text containing prose like `do not` was scanned as Ruby by
///   the nested-body override, so private methods after a heredoc were treated as
///   public. Heredoc line ranges are now ignored while scanning for enclosing
///   block/conditional openers.
/// - FP: the visibility walker only recursed through a small set of container
///   nodes, so it missed `private` for delegations nested under generic Prism
///   wrappers such as `CONST = Class.new do`, `class_exec do`, and `if ... class`.
///   Visibility now uses Prism's full visitor and checks every statement list,
///   while keeping RuboCop's sibling-scope rules intact.
pub struct Delegate;

impl Cop for Delegate {
    fn name(&self) -> &'static str {
        "Rails/Delegate"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["**/app/controllers/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CALL_NODE,
            CLASS_VARIABLE_READ_NODE,
            CONSTANT_PATH_NODE,
            CONSTANT_READ_NODE,
            DEF_NODE,
            GLOBAL_VARIABLE_READ_NODE,
            INSTANCE_VARIABLE_READ_NODE,
            LOCAL_VARIABLE_READ_NODE,
            REQUIRED_PARAMETER_NODE,
            SELF_NODE,
            STATEMENTS_NODE,
        ]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let enforce_for_prefixed = config.get_bool("EnforceForPrefixed", true);

        let def_node = match node.as_def_node() {
            Some(d) => d,
            None => return,
        };

        // Skip class/module methods (def self.foo)
        if def_node.receiver().is_some() {
            return;
        }

        // Collect parameter names (for argument forwarding check)
        let param_names: Vec<Vec<u8>> = if let Some(params) = def_node.parameters() {
            // Only support simple required positional parameters for forwarding
            let has_non_required = params.optionals().iter().next().is_some()
                || params.rest().is_some()
                || params.keywords().iter().next().is_some()
                || params.keyword_rest().is_some()
                || params.block().is_some();
            if has_non_required {
                return;
            }
            params
                .requireds()
                .iter()
                .filter_map(|p| {
                    p.as_required_parameter_node()
                        .map(|rp| rp.name().as_slice().to_vec())
                })
                .collect()
        } else {
            Vec::new()
        };

        // Body must be a single call expression.
        // For regular defs: body is StatementsNode with one statement.
        // For endless methods (def foo = expr): body is the call node directly.
        let body = match def_node.body() {
            Some(b) => b,
            None => return,
        };

        let call = if let Some(stmts) = body.as_statements_node() {
            let body_nodes: Vec<_> = stmts.body().iter().collect();
            if body_nodes.len() != 1 {
                return;
            }
            match body_nodes[0].as_call_node() {
                Some(c) => c,
                None => return,
            }
        } else {
            // Endless method: body is the call node directly
            match body.as_call_node() {
                Some(c) => c,
                None => return,
            }
        };

        // Check method name matching:
        // 1. Direct match: def foo; bar.foo; end
        // 2. Prefixed match (when EnforceForPrefixed): def bar_foo; bar.foo; end
        let def_name = def_node.name_loc().as_slice();
        let call_name = call.name().as_slice();

        // Must have a receiver (delegating to another object)
        let receiver = match call.receiver() {
            Some(r) => r,
            None => return,
        };

        let name_matches_directly = call_name == def_name;
        let name_matches_prefixed = if enforce_for_prefixed && !name_matches_directly {
            // Check if def_name == receiver_name + "_" + call_name
            // Skip prefix check for `self` receiver (RuboCop returns '' for self prefix)
            if receiver.as_self_node().is_some() {
                false
            } else {
                let recv_name = get_receiver_name(&receiver);
                if let Some(rn) = recv_name {
                    let mut expected = rn;
                    expected.push(b'_');
                    expected.extend_from_slice(call_name);
                    expected == def_name
                } else {
                    false
                }
            }
        } else {
            false
        };

        if !name_matches_directly && !name_matches_prefixed {
            return;
        }

        // Safe navigation (&.) is ignored — Rails' delegate with allow_nil
        // has different semantics than safe navigation
        if call
            .call_operator_loc()
            .is_some_and(|op: ruby_prism::Location<'_>| op.as_slice() == b"&.")
        {
            return;
        }

        // Receiver must be a delegatable target:
        // - Instance variable (@foo.bar → delegate :bar, to: :foo)
        // - Simple method/local variable (foo.bar → delegate :bar, to: :foo)
        // - Constant (Setting.bar → delegate :bar, to: :Setting)
        // - self (self.bar → delegate :bar, to: :self)
        // - self.class (self.class.bar → delegate :bar, to: :class)
        // - Class/global variable (@@var.bar, $var.bar)
        // NOT: literals, arbitrary chained calls, etc.
        let is_delegatable_receiver = if receiver.as_instance_variable_read_node().is_some()
            || receiver.as_self_node().is_some()
            || receiver.as_class_variable_read_node().is_some()
            || receiver.as_global_variable_read_node().is_some()
        {
            true
        } else if let Some(recv_call) = receiver.as_call_node() {
            // self.class → delegate to :class
            if recv_call.name().as_slice() == b"class"
                && recv_call
                    .receiver()
                    .is_some_and(|r| r.as_self_node().is_some())
                && recv_call.arguments().is_none()
            {
                true
            } else {
                // Simple receiverless method call (acts as a local variable)
                recv_call.receiver().is_none()
                    && recv_call.arguments().is_none()
                    && recv_call.block().is_none()
            }
        } else {
            receiver.as_constant_read_node().is_some() || receiver.as_constant_path_node().is_some()
        };

        if !is_delegatable_receiver {
            return;
        }

        // Check argument forwarding: call args must match def params 1:1
        let call_arg_names: Vec<Vec<u8>> = if let Some(args) = call.arguments() {
            args.arguments()
                .iter()
                .filter_map(|a| {
                    a.as_local_variable_read_node()
                        .map(|lv| lv.name().as_slice().to_vec())
                })
                .collect()
        } else {
            Vec::new()
        };

        // Argument count must match and all must be simple lvar forwards
        if call_arg_names.len() != param_names.len() {
            return;
        }
        let call_arg_count = if let Some(args) = call.arguments() {
            args.arguments().iter().count()
        } else {
            0
        };
        if call_arg_count != param_names.len() {
            return;
        }
        // Each param must forward to matching lvar in same order
        for (param, arg) in param_names.iter().zip(call_arg_names.iter()) {
            if param != arg {
                return;
            }
        }

        // Should not have a block
        if call.block().is_some() {
            return;
        }

        // When EnforceForPrefixed is false, skip prefixed delegations
        // (e.g., `def foo_bar; foo.bar; end` where method starts with receiver name)
        // Must check all receiver types, not just CallNode.
        if !enforce_for_prefixed && !name_matches_directly {
            // If the name only matched via prefix, skip it
            return;
        }

        // Skip private/protected/module_function methods — RuboCop only flags public methods.
        // Uses AST-based sibling analysis to determine visibility.
        if is_non_public_method(parse_result, node, def_name) {
            return;
        }

        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            "Use `delegate` to define delegations.".to_string(),
        ));
    }
}

/// AST-based check: is this def non-public?
///
/// Checks three patterns (matching RuboCop's VisibilityHelp):
/// 1. Inline modifier: `private def foo` (def is an argument to an access modifier call)
/// 2. Bare modifier: `private` / `protected` / `module_function` preceding the def in siblings
/// 3. Retroactive modifier: `private :foo` / `module_function :foo` after the def in siblings
fn is_non_public_method(
    parse_result: &ruby_prism::ParseResult<'_>,
    def_node: &ruby_prism::Node<'_>,
    method_name: &[u8],
) -> bool {
    let def_offset = def_node.location().start_offset();
    let mut checker = VisibilityChecker {
        def_offset,
        method_name,
        result: None,
        ancestor_has_module_function: false,
    };
    checker.visit(&parse_result.node());
    checker.result.unwrap_or(false)
}

/// Visitor that finds the def's sibling scope and checks visibility in one pass.
struct VisibilityChecker<'a> {
    def_offset: usize,
    method_name: &'a [u8],
    /// None = not found yet, Some(true) = non-public, Some(false) = public
    result: Option<bool>,
    /// True if any ancestor module/begin scope contains a `module_function` call.
    /// RuboCop's `module_function_declared?` checks ALL ancestors.
    ancestor_has_module_function: bool,
}

impl<'pr> VisibilityChecker<'_> {
    /// Check a list of sibling statements for the def's visibility.
    fn check_siblings(&mut self, stmts: impl Iterator<Item = ruby_prism::Node<'pr>>) -> bool {
        let stmts: Vec<_> = stmts.collect();

        // First check if our def is a direct child of this scope
        let has_our_def = stmts.iter().any(|s| self.contains_def_at(s));
        if !has_our_def {
            return false;
        }

        // Found the scope — compute visibility.
        // If any ancestor has module_function, the def is non-public.
        if self.ancestor_has_module_function {
            self.result = Some(true);
            return true;
        }

        let mut current_visibility = b"public".as_slice();
        let mut found_def = false;

        for sibling in &stmts {
            // Check inline modifier: `private def foo`
            if let Some(call) = sibling.as_call_node() {
                if access_modifier_predicates::is_access_modifier_declaration(&call) {
                    if let Some(args) = call.arguments() {
                        for arg in args.arguments().iter() {
                            if arg.as_def_node().is_some()
                                && arg.location().start_offset() == self.def_offset
                            {
                                self.result = Some(call.name().as_slice() != b"public");
                                return true;
                            }
                        }
                    }
                }
            }

            // Track bare modifiers before the def
            if sibling.location().start_offset() == self.def_offset {
                found_def = true;
                if current_visibility != b"public" {
                    self.result = Some(true);
                    return true;
                }
                continue;
            }

            if !found_def {
                if let Some(call) = sibling.as_call_node() {
                    if access_modifier_predicates::is_bare_access_modifier(&call) {
                        current_visibility = call.name().as_slice();
                    }
                }
            }

            // Retroactive modifier after the def
            if found_def {
                if let Some(call) = sibling.as_call_node() {
                    if access_modifier_predicates::is_non_bare_access_modifier(&call)
                        && call.name().as_slice() != b"public"
                        && self.has_single_symbol_arg_matching(&call)
                    {
                        self.result = Some(true);
                        return true;
                    }
                }
            }
        }

        self.result = Some(false);
        true
    }

    fn has_single_symbol_arg_matching(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        let Some(args) = call.arguments() else {
            return false;
        };
        let mut iter = args.arguments().iter();
        let Some(first) = iter.next() else {
            return false;
        };
        if iter.next().is_some() {
            return false;
        }
        first
            .as_symbol_node()
            .is_some_and(|sym| sym.unescaped() == self.method_name)
    }

    fn contains_def_at(&self, node: &ruby_prism::Node<'_>) -> bool {
        if node.as_def_node().is_some() && node.location().start_offset() == self.def_offset {
            return true;
        }
        if let Some(call) = node.as_call_node() {
            if let Some(args) = call.arguments() {
                for arg in args.arguments().iter() {
                    if arg.as_def_node().is_some()
                        && arg.location().start_offset() == self.def_offset
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check if a list of statements contains a bare or non-bare `module_function` call.
    fn has_module_function(stmts: impl Iterator<Item = ruby_prism::Node<'pr>>) -> bool {
        stmts.into_iter().any(|s| {
            s.as_call_node().is_some_and(|call| {
                access_modifier_predicates::is_access_modifier_declaration(&call)
                    && call.name().as_slice() == b"module_function"
            })
        })
    }

    fn body_has_module_function(body: &ruby_prism::Node<'pr>) -> bool {
        if let Some(stmts) = body.as_statements_node() {
            return Self::has_module_function(stmts.body().iter());
        }
        if let Some(begin) = body.as_begin_node() {
            return begin
                .statements()
                .is_some_and(|stmts| Self::has_module_function(stmts.body().iter()));
        }
        false
    }
}

impl<'pr> Visit<'pr> for VisibilityChecker<'_> {
    fn visit_program_node(&mut self, node: &ruby_prism::ProgramNode<'pr>) {
        if self.result.is_some() {
            return;
        }

        if !self.check_siblings(node.statements().body().iter()) {
            for stmt in node.statements().body().iter() {
                self.visit(&stmt);
                if self.result.is_some() {
                    break;
                }
            }
        }
    }

    fn visit_statements_node(&mut self, node: &ruby_prism::StatementsNode<'pr>) {
        if self.result.is_some() {
            return;
        }

        if !self.check_siblings(node.body().iter()) {
            for stmt in node.body().iter() {
                self.visit(&stmt);
                if self.result.is_some() {
                    break;
                }
            }
        }
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        if self.result.is_some() {
            return;
        }

        if let Some(stmts) = node.statements() {
            self.visit(&stmts.as_node());
        }
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        if self.result.is_some() {
            return;
        }

        let Some(body) = node.body() else {
            return;
        };

        let saved = self.ancestor_has_module_function;
        if Self::body_has_module_function(&body) {
            self.ancestor_has_module_function = true;
        }
        self.visit(&body);
        self.ancestor_has_module_function = saved;
    }
}

/// Extract the receiver name as bytes for prefix checking.
/// Returns None if the receiver type doesn't support prefix matching.
fn get_receiver_name(receiver: &ruby_prism::Node<'_>) -> Option<Vec<u8>> {
    if let Some(recv_call) = receiver.as_call_node() {
        if recv_call.receiver().is_none() {
            return Some(recv_call.name().as_slice().to_vec());
        }
        // self.class → prefix is "class"
        if recv_call
            .receiver()
            .is_some_and(|r| r.as_self_node().is_some())
            && recv_call.arguments().is_none()
        {
            return Some(recv_call.name().as_slice().to_vec());
        }
    }
    if let Some(iv) = receiver.as_instance_variable_read_node() {
        // ivar name includes @, e.g. @foo → prefix is "@foo"
        return Some(iv.name().as_slice().to_vec());
    }
    if let Some(cv) = receiver.as_class_variable_read_node() {
        return Some(cv.name().as_slice().to_vec());
    }
    if let Some(gv) = receiver.as_global_variable_read_node() {
        return Some(gv.name().as_slice().to_vec());
    }
    if let Some(cr) = receiver.as_constant_read_node() {
        return Some(cr.name().as_slice().to_vec());
    }
    if receiver.as_constant_path_node().is_some() {
        // For ConstantPathNode, extract source text
        let loc = receiver.location();
        return Some(loc.as_slice().to_vec());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(Delegate, "cops/rails/delegate");
}
