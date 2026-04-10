use std::collections::HashSet;

use ruby_prism::Visit;

use crate::cop::shared::method_identifier_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// RuboCop parity notes:
/// - Local variables are tracked in source order (not pre-scanned). `self.x` before
///   `x = ...` is flagged as redundant, matching RuboCop's lazy variable tracking.
/// - `if`/`unless`/`while`/`until` nodes prescan descendants (including inside
///   blocks but not into nested defs/classes/modules) for local variable
///   assignments. This makes `self.x` in the condition allowed when `x` is
///   assigned anywhere inside the conditional body, even in nested blocks.
/// - Nested block and lambda locals leak forward into the enclosing scope for later
///   disambiguation, so `self.x` stays allowed after an earlier `do |x| ... end` or
///   `->(x) { ... }`, but not before that nested scope appears.
/// - Compound self-assignments (`self.count += 1`, `self.count ||= 1`, `self.count &&= 1`)
///   make later `self.count` reads acceptable in source order, even across later methods
///   and class/module nesting. Plain setters like `self.value = 1` do not.
/// - Parameter default values are visited for `self.` checks. `def foo(x = self.bar)`
///   flags `self.bar` unless `bar` is also a parameter name.
/// - Operator methods (`+`, `-`, `<<`, `==`, etc.) called with dot syntax
///   (`self.+(other)`, `self.<<(item)`) are not flagged, matching RuboCop's
///   `operator_method?` check.
/// - Explicit call syntax `self.(...)` is allowed. Prism exposes it as a
///   `CallNode` named `call` whose opening `(` starts immediately after the
///   dot, so it must be distinguished from ordinary `self.call(...)`.
/// - `self::foo(...)` is checked the same as `self.foo(...)`, but `pp` is not a
///   Kernel-method exemption. RuboCop flags `self.pp(...)` while still allowing
///   real Kernel methods like `self.open(...)`, `self.eval(...)`, `self.test(...)`,
///   and `self.printf(...)`.
/// - Prism exposes splatted multi-write targets like `*self.arr = value` as a
///   `CallTargetNode` nested under `SplatNode` instead of a normal `CallNode`, so
///   they need a dedicated check to keep offense fixtures aligned with RuboCop.
/// - Scope boundaries (def, class, module) prevent local variables from leaking
///   across them. A lambda param `token` at class body level does not suppress
///   detection of `self.token` inside a method definition. Blocks within a def
///   can still see the def's locals.
/// - Fixed class-body leakage: block/lambda locals introduced directly under a
///   class/module/root body do not leak into later sibling callbacks or lambdas
///   (`scope { |profile| ... }` must not suppress later `self.profile` in
///   `code_numbering ... -> { self.profile }`). They still leak within defs and
///   enclosing blocks where RuboCop keeps them visible.
/// - `rescue => self.foo` references are detected via `visit_rescue_node`. Prism
///   calls `visit_rescue_node` directly from `visit_begin_node`, bypassing the
///   normal branch dispatch, so a dedicated visitor is needed to catch these.
/// - Nested defs inherit locals from all enclosing scopes when there is an
///   enclosing Method or Soft scope. This matches RuboCop's `add_scope` model
///   where the def NODE retains the outer shared array, making ancestor-walk
///   find outer locals through it (`def klass.inherited(child)` inside a method
///   that has `defaults` as a local sees it via the inherited scope).
/// - Destructured block parameters (`|(title, path), i|`) introduce each
///   nested name into the scope, so `self.path` inside the block is allowed.
/// - Compound self-assignment value expressions: Prism's `CallOrWriteNode`,
///   `CallAndWriteNode`, and `CallOperatorWriteNode` contain an embedded read
///   `CallNode` for the method being assigned. The compound target is added to
///   `allowed_self_methods` before visiting the node so that `self.x` on the
///   RHS of `self.x ||= default` is not flagged.
/// - `class << self` uses Soft scoping when inside a Method or Soft scope
///   (matching RuboCop's lack of an `on_sclass` handler). This lets enclosing
///   method params remain visible through the singleton class boundary, e.g.
///   `def build(table_name); class << self; define_method(:search) { |q|
///   self.table_name }; end; end`.
/// - Block/lambda locals propagate through class/module (ClassLike) scope
///   boundaries when an enclosing block or method scope exists beyond the
///   ClassLike parent. This matches RuboCop's flat shared-array model where
///   `describe Foo do; class Bar; def m; self.x; end; end; end` sees lambda
///   params from inside the describe block. At the top level (only Root above
///   ClassLike) the merge is skipped to prevent class-body leakage.
/// - `unless ... else` branch visitation follows parser's normalized `if`
///   semantics, not Prism source order: RuboCop visits the source `else`
///   branch before the source `unless` body. This keeps block params or locals
///   introduced only in the source `unless` body from suppressing later
///   offenses in the source `else` branch (for example, `debug? self.actor`
///   after `actors.each do |actor|` in the `unless` body).
pub struct RedundantSelf;

/// Methods where self. is always required (Ruby keywords).
const ALLOWED_METHODS: &[&[u8]] = &[
    b"alias",
    b"and",
    b"begin",
    b"break",
    b"case",
    b"class",
    b"def",
    b"defined?",
    b"do",
    b"else",
    b"elsif",
    b"end",
    b"ensure",
    b"false",
    b"for",
    b"if",
    b"in",
    b"module",
    b"next",
    b"nil",
    b"not",
    b"or",
    b"redo",
    b"rescue",
    b"retry",
    b"return",
    b"self",
    b"super",
    b"then",
    b"true",
    b"undef",
    b"unless",
    b"until",
    b"when",
    b"while",
    b"yield",
    b"__FILE__",
    b"__LINE__",
    b"__ENCODING__",
    // raise is commonly treated as keyword-like
    b"raise",
];

/// Kernel methods where self. is required to avoid ambiguity with top-level functions.
const KERNEL_METHODS: &[&[u8]] = &[
    b"eval",
    b"open",
    b"puts",
    b"print",
    b"p",
    b"warn",
    b"fail",
    b"sleep",
    b"exit",
    b"exit!",
    b"abort",
    b"at_exit",
    b"fork",
    b"exec",
    b"system",
    b"spawn",
    b"rand",
    b"srand",
    b"gets",
    b"readline",
    b"readlines",
    b"select",
    b"format",
    b"sprintf",
    b"printf",
    b"putc",
    b"loop",
    b"require",
    b"require_relative",
    b"load",
    b"autoload",
    b"autoload?",
    b"binding",
    b"block_given?",
    b"iterator?",
    b"caller",
    b"caller_locations",
    b"catch",
    b"throw",
    b"global_variables",
    b"local_variables",
    b"set_trace_func",
    b"test",
    b"trace_var",
    b"untrace_var",
    b"trap",
    b"lambda",
    b"proc",
    b"Array",
    b"Complex",
    b"Float",
    b"Hash",
    b"Integer",
    b"Rational",
    b"String",
    b"__callee__",
    b"__dir__",
    b"__method__",
];

/// Returns true if the method name starts with an uppercase letter,
/// which could be confused with a constant reference.
fn is_uppercase_method(name: &[u8]) -> bool {
    name.first().is_some_and(|&b| b.is_ascii_uppercase())
}

impl Cop for RedundantSelf {
    fn name(&self) -> &'static str {
        "Style/RedundantSelf"
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let mut visitor = RedundantSelfVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            local_scopes: vec![(HashSet::new(), ScopeKind::Root)],
            allowed_self_methods: HashSet::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

/// Distinguishes method scopes, class/module/root scopes, and soft/transparent
/// ones (block, lambda). This keeps block locals leaking forward where RuboCop
/// does (within defs and enclosing blocks) without letting class-body lambda
/// params suppress later sibling class-body callbacks.
#[derive(Clone, Copy, PartialEq)]
enum ScopeKind {
    /// root — top-level locals are not a shared "forward leak" scope for later
    /// sibling blocks in RuboCop's implementation.
    Root,
    /// def — block locals may leak forward into the surrounding method scope.
    Method,
    /// class, module, singleton_class — block locals should not leak into later
    /// sibling class-body statements.
    ClassLike,
    /// block, lambda — variables are visible through this boundary.
    Soft,
}

struct RedundantSelfVisitor<'a> {
    cop: &'a RedundantSelf,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    /// Stack of local variable scopes. Each method/block introduces a new scope.
    /// The `ScopeKind` determines whether the scope acts as a search boundary.
    local_scopes: Vec<(HashSet<Vec<u8>>, ScopeKind)>,
    /// Method names where `self.x` is allowed because a prior compound assignment
    /// (`self.x ||=`, `self.x &&=`, `self.x +=`, etc.) appeared earlier in the
    /// current enclosing file/class/module. This matches RuboCop's source-order
    /// accumulation across later methods, while still excluding plain setters.
    allowed_self_methods: HashSet<Vec<u8>>,
}

impl RedundantSelfVisitor<'_> {
    fn add_local(&mut self, name: &[u8]) {
        if let Some((scope, _)) = self.local_scopes.last_mut() {
            scope.insert(name.to_vec());
        }
    }

    fn is_local_variable(&self, name: &[u8]) -> bool {
        // Search from innermost scope outward. Allow at most one non-soft scope
        // (the enclosing def/class/module/root). A second non-soft boundary
        // means we've crossed a scope wall (e.g., class body -> def), so we
        // stop. This prevents class-level locals from leaking into defs while
        // still letting blocks within a def or enclosing block see those locals.
        let mut hard_seen = false;
        for (scope, kind) in self.local_scopes.iter().rev() {
            if *kind != ScopeKind::Soft {
                if hard_seen {
                    break;
                }
                hard_seen = true;
            }
            if scope.contains(name) {
                return true;
            }
        }
        false
    }

    fn add_allowed_self_method(&mut self, name: &[u8]) {
        self.allowed_self_methods.insert(name.to_vec());
    }

    fn is_allowed_self_method(&self, name: &[u8]) -> bool {
        self.allowed_self_methods.contains(name)
    }

    fn collect_multi_target_params(&mut self, mt: &ruby_prism::MultiTargetNode<'_>) {
        for target in mt.lefts().iter() {
            if let Some(rp) = target.as_required_parameter_node() {
                self.add_local(rp.name().as_slice());
            } else if let Some(inner) = target.as_multi_target_node() {
                self.collect_multi_target_params(&inner);
            }
        }
        if let Some(rest) = mt.rest() {
            if let Some(splat) = rest.as_splat_node() {
                if let Some(expr) = splat.expression() {
                    if let Some(rp) = expr.as_required_parameter_node() {
                        self.add_local(rp.name().as_slice());
                    }
                }
            }
        }
        for target in mt.rights().iter() {
            if let Some(rp) = target.as_required_parameter_node() {
                self.add_local(rp.name().as_slice());
            } else if let Some(inner) = target.as_multi_target_node() {
                self.collect_multi_target_params(&inner);
            }
        }
    }

    fn collect_params_from_node(&mut self, params: &ruby_prism::ParametersNode<'_>) {
        for p in params.requireds().iter() {
            if let Some(req) = p.as_required_parameter_node() {
                self.add_local(req.name().as_slice());
            } else if let Some(mt) = p.as_multi_target_node() {
                self.collect_multi_target_params(&mt);
            }
        }
        for p in params.optionals().iter() {
            if let Some(op) = p.as_optional_parameter_node() {
                self.add_local(op.name().as_slice());
            }
        }
        if let Some(rest) = params.rest() {
            if let Some(rp) = rest.as_rest_parameter_node() {
                if let Some(name) = rp.name() {
                    self.add_local(name.as_slice());
                }
            }
        }
        for p in params.keywords().iter() {
            if let Some(kw) = p.as_required_keyword_parameter_node() {
                self.add_local(kw.name().as_slice());
            } else if let Some(kw) = p.as_optional_keyword_parameter_node() {
                self.add_local(kw.name().as_slice());
            }
        }
        if let Some(kw_rest) = params.keyword_rest() {
            if let Some(kw_rest_param) = kw_rest.as_keyword_rest_parameter_node() {
                if let Some(name) = kw_rest_param.name() {
                    self.add_local(name.as_slice());
                }
            }
        }
        if let Some(block) = params.block() {
            if let Some(name) = block.name() {
                self.add_local(name.as_slice());
            }
        }
    }

    /// Apply the results of a conditional prescan to the current scope.
    fn apply_conditional_prescan(&mut self, scanner: ConditionalLocalScanner) {
        for name in scanner.names {
            self.add_local(&name);
        }
    }

    fn merge_current_scope_into_parent(&mut self) {
        if self.local_scopes.len() < 2 {
            return;
        }

        // Check parent kind and whether there's an enclosing Soft/Method
        // before taking a mutable borrow.
        let parent_kind = self.local_scopes[self.local_scopes.len() - 2].1;
        let should_merge = if matches!(parent_kind, ScopeKind::Method | ScopeKind::Soft) {
            true
        } else if parent_kind == ScopeKind::ClassLike {
            // RuboCop has no on_class/on_module/on_sclass handlers, so
            // block/lambda locals propagate through class/module boundaries
            // when an enclosing block or method scope exists (e.g., a
            // `describe` block wrapping a class). At the top level the
            // ClassLike parent has only Root above it, so the merge is
            // correctly skipped — preventing class-body locals from
            // leaking into sibling methods.
            let len = self.local_scopes.len();
            len > 2
                && self.local_scopes[..len - 2]
                    .iter()
                    .any(|(_, k)| matches!(k, ScopeKind::Method | ScopeKind::Soft))
        } else {
            false
        };

        let (current_scope, _) = self.local_scopes.pop().unwrap();
        if should_merge {
            if let Some((parent_scope, _)) = self.local_scopes.last_mut() {
                parent_scope.extend(current_scope);
            }
        }
    }

    fn is_explicit_call_syntax(
        &self,
        node: &ruby_prism::CallNode<'_>,
        call_op: &ruby_prism::Location<'_>,
        name_bytes: &[u8],
    ) -> bool {
        name_bytes == b"call"
            && call_op.as_slice() == b"."
            && node
                .opening_loc()
                .is_some_and(|opening| opening.start_offset() == call_op.end_offset())
    }

    fn self_receiver_is_redundant(
        &self,
        call_op: &ruby_prism::Location<'_>,
        name_bytes: &[u8],
        explicit_call_syntax: bool,
    ) -> bool {
        let is_setter = name_bytes.ends_with(b"=")
            && name_bytes != b"=="
            && name_bytes != b"!="
            && name_bytes != b"<="
            && name_bytes != b">="
            && name_bytes != b"===";

        matches!(call_op.as_slice(), b"." | b"::")
            && !explicit_call_syntax
            && !is_setter
            && name_bytes != b"[]"
            && name_bytes != b"[]="
            && !method_identifier_predicates::is_operator_method(name_bytes)
            && !ALLOWED_METHODS.contains(&name_bytes)
            && !KERNEL_METHODS.contains(&name_bytes)
            && !is_uppercase_method(name_bytes)
            && !self.is_local_variable(name_bytes)
            && !self.is_allowed_self_method(name_bytes)
    }

    fn add_redundant_self_offense(&mut self, self_loc: ruby_prism::Location<'_>) {
        let (line, column) = self.source.offset_to_line_col(self_loc.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Redundant `self` detected.".to_string(),
        ));
    }
}

/// Prescan visitor for conditional nodes (`if`/`unless`/`while`/`until`).
/// Collects local variable names from descendants, descending into blocks
/// and lambdas (whose variables leak into the enclosing scope) but stopping
/// at defs, classes, and modules (which create isolated variable scopes).
/// This prevents modifier conditionals like `def foo; ...; end if cond` from
/// leaking method-local variables into the enclosing scope.
struct ConditionalLocalScanner {
    names: Vec<Vec<u8>>,
}

impl<'pr> Visit<'pr> for ConditionalLocalScanner {
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        self.names.push(node.name().as_slice().to_vec());
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_local_variable_target_node(
        &mut self,
        node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        self.names.push(node.name().as_slice().to_vec());
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        self.names.push(node.name().as_slice().to_vec());
        ruby_prism::visit_local_variable_or_write_node(self, node);
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        self.names.push(node.name().as_slice().to_vec());
        ruby_prism::visit_local_variable_and_write_node(self, node);
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        self.names.push(node.name().as_slice().to_vec());
        ruby_prism::visit_local_variable_operator_write_node(self, node);
    }

    // Stop at scope boundaries that create new local variable scopes.
    // Variables inside defs/classes/modules don't leak into the enclosing scope.
    // Blocks and lambdas are NOT stopped because their variables DO leak into
    // the enclosing method scope in Ruby.
    fn visit_def_node(&mut self, _node: &ruby_prism::DefNode<'pr>) {}
    fn visit_class_node(&mut self, _node: &ruby_prism::ClassNode<'pr>) {}
    fn visit_module_node(&mut self, _node: &ruby_prism::ModuleNode<'pr>) {}
    fn visit_singleton_class_node(&mut self, _node: &ruby_prism::SingletonClassNode<'pr>) {}
}

impl<'pr> Visit<'pr> for RedundantSelfVisitor<'_> {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        // Inherit locals from enclosing scopes for nested defs.
        // RuboCop's `add_scope` sets all descendants of a def/block to a shared
        // array; when a nested def calls `add_scope`, the nested def NODE itself
        // retains the outer shared array. The ancestor walk in `allowed_send_node?`
        // finds outer locals through this retained reference. We replicate this by
        // pre-populating the new scope with all enclosing locals — but only when
        // there is an enclosing Method or Soft (block) scope, so that top-level
        // class-body locals (which RuboCop doesn't propagate through class nodes)
        // don't leak in.
        let mut inherited_locals = HashSet::new();
        let mut has_enclosing_method_or_soft = false;
        for (scope, kind) in self.local_scopes.iter().rev() {
            inherited_locals.extend(scope.iter().cloned());
            if matches!(kind, ScopeKind::Method | ScopeKind::Soft) {
                has_enclosing_method_or_soft = true;
            }
        }

        let initial_scope = if has_enclosing_method_or_soft {
            inherited_locals
        } else {
            HashSet::new()
        };

        self.local_scopes.push((initial_scope, ScopeKind::Method));

        if let Some(params) = node.parameters() {
            // Collect parameter names into scope first (before visiting defaults).
            // This ensures `def foo(x = self.x)` sees `x` as a local, matching RuboCop.
            self.collect_params_from_node(&params);

            // Visit parameter default value expressions — they may contain `self.` calls
            // that should be checked for redundancy.
            for p in params.optionals().iter() {
                if let Some(op) = p.as_optional_parameter_node() {
                    self.visit(&op.value());
                }
            }
            for p in params.keywords().iter() {
                if let Some(kw) = p.as_optional_keyword_parameter_node() {
                    self.visit(&kw.value());
                }
            }
        }

        // No prescan — locals are tracked in visit order, matching RuboCop's
        // lazy variable tracking. `self.x` before `x = ...` is flagged.
        if let Some(body) = node.body() {
            self.visit(&body);
        }

        self.local_scopes.pop();
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            if receiver.as_self_node().is_some() {
                if let Some(call_op) = node.call_operator_loc() {
                    if matches!(call_op.as_slice(), b"." | b"::") {
                        let method_name = node.name();
                        let name_bytes = method_name.as_slice();
                        let explicit_call_syntax =
                            self.is_explicit_call_syntax(node, &call_op, name_bytes);
                        if self.self_receiver_is_redundant(
                            &call_op,
                            name_bytes,
                            explicit_call_syntax,
                        ) {
                            self.add_redundant_self_offense(receiver.location());
                        }
                    }
                }
            }
        }

        // Visit receiver (for chained calls like self.name.demodulize — we need to
        // check the inner self.name), arguments, and block.
        if let Some(receiver) = node.receiver() {
            // Only visit non-self receivers (self is already handled above)
            if receiver.as_self_node().is_none() {
                self.visit(&receiver);
            }
        }
        if let Some(args) = node.arguments() {
            for arg in args.arguments().iter() {
                self.visit(&arg);
            }
        }
        if let Some(block) = node.block() {
            self.visit(&block);
        }
    }

    // Class/module bodies have a different `self` context.
    // We still need to visit them to find `self.` calls within method definitions.
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        // Push a new scope for the class body (local variables from the enclosing scope
        // are not visible inside a class body).
        self.local_scopes
            .push((HashSet::new(), ScopeKind::ClassLike));
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.local_scopes.pop();
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        self.local_scopes
            .push((HashSet::new(), ScopeKind::ClassLike));
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.local_scopes.pop();
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        // RuboCop has no `on_sclass` handler, so `class << self` is transparent
        // to scoping inside methods/blocks. Use Soft when an enclosing Method or
        // Soft scope exists so that the enclosing method's locals remain visible
        // through the singleton class boundary. At the top level or inside a
        // class body, keep ClassLike to prevent unwanted leakage.
        let use_soft = self
            .local_scopes
            .iter()
            .rev()
            .any(|(_, k)| matches!(k, ScopeKind::Method | ScopeKind::Soft));
        let kind = if use_soft {
            ScopeKind::Soft
        } else {
            ScopeKind::ClassLike
        };
        self.local_scopes.push((HashSet::new(), kind));
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        if use_soft {
            self.merge_current_scope_into_parent();
        } else {
            self.local_scopes.pop();
        }
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        // Block parameters shadow method names — `self.x` is required for
        // disambiguation when a block parameter `x` is in scope.
        // Push a new scope for block params (they are block-local).
        // Soft boundary: variables are visible through blocks from enclosing defs.
        self.local_scopes.push((HashSet::new(), ScopeKind::Soft));

        if let Some(params) = node.parameters() {
            if let Some(block_params) = params.as_block_parameters_node() {
                if let Some(inner_params) = block_params.parameters() {
                    self.collect_params_from_node(&inner_params);
                }
            }
        }

        if let Some(body) = node.body() {
            self.visit(&body);
        }

        self.merge_current_scope_into_parent();
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        // Lambda parameters work the same as block parameters for scoping.
        self.local_scopes.push((HashSet::new(), ScopeKind::Soft));

        if let Some(params) = node.parameters() {
            if let Some(block_params) = params.as_block_parameters_node() {
                if let Some(inner_params) = block_params.parameters() {
                    self.collect_params_from_node(&inner_params);
                }
            }
        }

        if let Some(body) = node.body() {
            self.visit(&body);
        }

        self.merge_current_scope_into_parent();
    }

    // --- Local variable tracking in visit order (replaces prescan) ---

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        // Add local BEFORE visiting value — matches RuboCop where `x = self.x`
        // does NOT flag self.x (x is already in scope on the RHS).
        self.add_local(node.name().as_slice());
        self.visit(&node.value());
    }

    fn visit_local_variable_target_node(
        &mut self,
        node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        self.add_local(node.name().as_slice());
        // No children to visit
        let _ = node;
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        self.add_local(node.name().as_slice());
        self.visit(&node.value());
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        self.add_local(node.name().as_slice());
        self.visit(&node.value());
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        self.add_local(node.name().as_slice());
        self.visit(&node.value());
    }

    // --- Conditional prescan: if/unless/while/until ---
    // RuboCop's on_if scans all descendants (including inside blocks) for lvasgn
    // and adds those variable names to the scope before visiting. This makes
    // `self.x` allowed in the condition when `x` is assigned anywhere in the body.

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        let mut scanner = ConditionalLocalScanner { names: Vec::new() };
        ruby_prism::visit_if_node(&mut scanner, node);
        self.apply_conditional_prescan(scanner);
        ruby_prism::visit_if_node(self, node);
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        let mut scanner = ConditionalLocalScanner { names: Vec::new() };
        ruby_prism::visit_unless_node(&mut scanner, node);
        self.apply_conditional_prescan(scanner);

        // Match parser/RuboCop's normalized `if` AST for `unless`: the source
        // `else` branch is the truthy branch and is visited before the source
        // `unless` body. Source-order visitation would incorrectly let locals
        // from the `unless` body leak into the `else` branch.
        self.visit(&node.predicate());
        if let Some(else_clause) = node.else_clause() {
            self.visit_else_node(&else_clause);
        }
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        let mut scanner = ConditionalLocalScanner { names: Vec::new() };
        ruby_prism::visit_while_node(&mut scanner, node);
        self.apply_conditional_prescan(scanner);
        ruby_prism::visit_while_node(self, node);
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        let mut scanner = ConditionalLocalScanner { names: Vec::new() };
        ruby_prism::visit_until_node(&mut scanner, node);
        self.apply_conditional_prescan(scanner);
        ruby_prism::visit_until_node(self, node);
    }

    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'pr>) {
        // Add to allowed BEFORE visiting children. Prism exposes the read side
        // of `self.x ||= expr(self.x)` as a separate CallNode inside the value
        // expression. RuboCop (parser gem) doesn't have this extra node, so it
        // never fires `on_send` for the inner read. Pre-allowing prevents a FP.
        if let Some(receiver) = node.receiver() {
            if receiver.as_self_node().is_some() {
                self.add_allowed_self_method(node.read_name().as_slice());
            }
        }
        ruby_prism::visit_call_or_write_node(self, node);
    }

    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            if receiver.as_self_node().is_some() {
                self.add_allowed_self_method(node.read_name().as_slice());
            }
        }
        ruby_prism::visit_call_and_write_node(self, node);
    }

    fn visit_call_operator_write_node(&mut self, node: &ruby_prism::CallOperatorWriteNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            if receiver.as_self_node().is_some() {
                self.add_allowed_self_method(node.read_name().as_slice());
            }
        }
        ruby_prism::visit_call_operator_write_node(self, node);
    }

    fn visit_splat_node(&mut self, node: &ruby_prism::SplatNode<'pr>) {
        if let Some(expression) = node.expression() {
            if let Some(call_target) = expression.as_call_target_node() {
                let receiver = call_target.receiver();
                if receiver.as_self_node().is_some()
                    && self.self_receiver_is_redundant(
                        &call_target.call_operator_loc(),
                        call_target.message_loc().as_slice(),
                        false,
                    )
                {
                    self.add_redundant_self_offense(receiver.location());
                }
            }
        }

        ruby_prism::visit_splat_node(self, node);
    }

    // Prism's RescueNode is visited via visit_rescue_node() which bypasses the
    // normal visit_begin_node branch dispatch. We need to handle it explicitly
    // to catch `self.` in rescue reference expressions like `rescue => self.foo`.
    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        // Check the reference (e.g., `rescue => self.foo` — the `self.foo` part)
        if let Some(reference) = node.reference() {
            // In `rescue => self.foo`, the reference is a CallTargetNode with a
            // SelfNode receiver. We need to check if `self.foo` is redundant.
            if let Some(call_target) = reference.as_call_target_node() {
                let receiver = call_target.receiver();
                if receiver.as_self_node().is_some()
                    && self.self_receiver_is_redundant(
                        &call_target.call_operator_loc(),
                        call_target.message_loc().as_slice(),
                        false,
                    )
                {
                    self.add_redundant_self_offense(receiver.location());
                }
            }
        }
        // Continue walking children
        ruby_prism::visit_rescue_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(RedundantSelf, "cops/style/redundant_self");
}
