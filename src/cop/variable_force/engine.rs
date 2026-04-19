//! The VariableForce AST visitor engine.
//!
//! Performs a single walk of the Prism AST, building a VariableTable and
//! dispatching hook callbacks to registered consumers at scope entry/exit
//! and variable declaration events.

use ruby_prism::Visit;

use super::VariableForceConsumer;
use super::assignment::{Assignment, AssignmentKind};
use super::reference::Reference;
use super::scope::ScopeKind;
use super::variable::DeclarationKind;
use super::variable_table::VariableTable;
use crate::cop::CopConfig;
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// A registered consumer with its config.
pub struct RegisteredConsumer<'a> {
    pub consumer: &'a dyn VariableForceConsumer,
    pub config: &'a CopConfig,
}

/// A branch context represents a single child of a conditional control
/// structure. Two branches are "exclusive" if they share the same
/// `parent_id` but have different `child_index` (e.g., if-then vs if-else).
#[derive(Debug, Clone)]
pub struct BranchContext {
    /// Unique ID for this branch context.
    pub id: usize,
    /// Scope index active when the branch was entered. Variables declared in a
    /// nested scope must ignore outer-scope branch contexts.
    pub scope_index: usize,
    /// ID of the parent conditional node (e.g., the IfNode). Used to
    /// determine if two branches belong to the same conditional.
    pub parent_id: u64,
    /// Which child of the conditional this branch is (0=then, 1=else, etc.).
    pub child_index: usize,
    /// Predicate-assignment contexts execute before the guarded branch and
    /// must stay visible to reads in that branch.
    pub predicate_context: bool,
    /// Modifier-form conditionals (`body if (x = ...)`, `body while (x = ...)`)
    /// must keep walking to an older assignment so the variable is in scope on
    /// the left side of the keyword, matching RuboCop's
    /// `in_modifier_conditional?` special-case.
    pub modifier_conditional: bool,
    /// Exception-handler main bodies (`begin` under rescue/ensure) can exit
    /// into sibling branches, so their writes are not exclusive with later
    /// rescue/else/ensure reads.
    pub may_jump_to_other_branch: bool,
    /// Exception-handler main bodies may terminate before later writes run, so
    /// references must keep walking past them instead of consuming the branch.
    pub may_run_incompletely: bool,
    /// Short-circuit logical branches (`&&`, `||`, `and`, `or`) stay
    /// reportable for ShadowedArgument even though VF must model them as
    /// branches for assignment liveness.
    pub short_circuit: bool,
}

/// The VariableForce engine. Walks the Prism AST and builds a complete
/// variable-scope model, dispatching hooks to consumers.
pub struct Engine<'a> {
    pub table: VariableTable,
    source: &'a SourceFile,
    consumers: &'a [RegisteredConsumer<'a>],
    diagnostics: Vec<Diagnostic>,
    /// Monotonically increasing counter for temporal ordering.
    sequence: usize,
    /// Depth inside conditional/branch constructs (if, unless, case, while,
    /// until, rescue, block, lambda). Assignments created while > 0 are
    /// marked `in_branch = true`.
    branch_depth: usize,
    /// All branch contexts created during this engine run, indexed by their
    /// `id`. Used to determine exclusivity between branches.
    branch_contexts: Vec<BranchContext>,
    /// Monotonically increasing counter for branch context IDs.
    next_branch_id: usize,
    /// Stack of active branch context IDs. The top is the current branch.
    branch_stack: Vec<usize>,
}

impl<'a> Engine<'a> {
    pub fn new(source: &'a SourceFile, consumers: &'a [RegisteredConsumer<'a>]) -> Self {
        Self {
            table: VariableTable::new(),
            source,
            consumers,
            diagnostics: Vec::new(),
            sequence: 0,
            branch_depth: 0,
            branch_contexts: Vec::new(),
            next_branch_id: 0,
            branch_stack: Vec::new(),
        }
    }

    fn next_sequence(&mut self) -> usize {
        let seq = self.sequence;
        self.sequence += 1;
        seq
    }

    fn branch_parent_id(location: &ruby_prism::Location<'_>) -> u64 {
        ((location.start_offset() as u64) << 32) ^ (location.end_offset() as u64)
    }

    /// Push a new branch context for a child of a conditional node.
    /// `parent_id` identifies the conditional node (use its start offset),
    /// `child_index` identifies which child (0=then, 1=else, etc.).
    fn push_branch(&mut self, parent_id: u64, child_index: usize, predicate_context: bool) {
        self.push_branch_with_flags(
            parent_id,
            child_index,
            predicate_context,
            false,
            false,
            false,
            false,
        );
    }

    #[allow(clippy::too_many_arguments)] // internal branch-context helper threading independent flags
    fn push_branch_with_flags(
        &mut self,
        parent_id: u64,
        child_index: usize,
        predicate_context: bool,
        modifier_conditional: bool,
        may_jump_to_other_branch: bool,
        may_run_incompletely: bool,
        short_circuit: bool,
    ) {
        let id = self.next_branch_id;
        self.next_branch_id += 1;
        let context = BranchContext {
            id,
            scope_index: self.table.current_scope_index(),
            parent_id,
            child_index,
            predicate_context,
            modifier_conditional,
            may_jump_to_other_branch,
            may_run_incompletely,
            short_circuit,
        };
        self.branch_contexts.push(context.clone());
        // Keep the live VariableTable copy in sync so reference tracking can
        // distinguish exclusive sibling branches during AST traversal.
        self.table.branch_contexts.push(context);
        self.branch_stack.push(id);
    }

    /// Pop the current branch context.
    fn pop_branch(&mut self) {
        self.branch_stack.pop();
    }

    /// The current branch ID, if inside a branch.
    fn current_branch_id(&self) -> Option<usize> {
        self.branch_stack.last().copied()
    }

    fn current_branch_path(&self) -> Vec<usize> {
        self.branch_stack.clone()
    }

    fn current_shadowing_in_branch(&self) -> bool {
        if self.branch_depth == 0 {
            return false;
        }

        // Some conditional contexts (currently case/case-match predicates)
        // increment branch_depth without pushing a branch context. They still
        // count as conditional for ShadowedArgument.
        if self.branch_depth > self.branch_stack.len() {
            return true;
        }

        self.branch_stack.iter().any(|&id| {
            self.branch_contexts
                .get(id)
                .is_some_and(|context| !context.short_circuit)
        })
    }

    /// Check if two branch IDs are mutually exclusive (belong to the same
    /// conditional parent but are different children).
    pub fn branches_exclusive(&self, a: Option<usize>, b: Option<usize>) -> bool {
        let (a_id, b_id) = match (a, b) {
            (Some(a), Some(b)) => (a, b),
            _ => return false,
        };
        if a_id == b_id {
            return false;
        }
        let a_ctx = &self.branch_contexts[a_id];
        let b_ctx = &self.branch_contexts[b_id];
        if a_ctx.predicate_context || b_ctx.predicate_context {
            return false;
        }
        a_ctx.parent_id == b_ctx.parent_id && a_ctx.child_index != b_ctx.child_index
    }

    /// Mark assignments as referenced for loop back-edges.
    ///
    /// After processing a loop body, walk all variables accessible in the
    /// current scope. For each variable that has BOTH an assignment AND a
    /// reference within the loop's offset range, mark the last such
    /// assignment as referenced (the next iteration may use it).
    ///
    /// Also marks branched assignments within the loop as referenced, since
    /// branches inside a loop may execute in a different iteration.
    fn mark_loop_back_edges(&mut self, loop_start: usize, loop_end: usize) {
        // Collect variable names that are referenced within the loop range.
        let mut referenced_names: Vec<Vec<u8>> = Vec::new();
        for scope in self.table.accessible_scopes() {
            for (name, var) in &scope.variables {
                let has_ref_in_loop = var
                    .references
                    .iter()
                    .any(|r| r.node_offset >= loop_start && r.node_offset < loop_end);
                if has_ref_in_loop {
                    referenced_names.push(name.clone());
                }
            }
        }

        // For each referenced variable, find assignments within the loop
        // and mark the last one as referenced.
        for name in &referenced_names {
            if let Some(var) = self.table.find_variable_mut(name) {
                let loop_assignments: Vec<usize> = var
                    .assignments
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| a.node_offset >= loop_start && a.node_offset < loop_end)
                    .map(|(i, _)| i)
                    .collect();

                if loop_assignments.is_empty() {
                    continue;
                }

                // Mark branched assignments in the loop as referenced
                for &idx in &loop_assignments {
                    if var.assignments[idx].in_branch {
                        var.assignments[idx].referenced = true;
                    }
                }

                // Mark the last assignment in the loop as referenced
                if let Some(&last_idx) = loop_assignments.last() {
                    var.assignments[last_idx].referenced = true;
                }
            }
        }
    }

    /// Run the engine on a parsed program node.
    pub fn run(&mut self, parse_result: &ruby_prism::ParseResult<'_>) {
        let root = parse_result.node();
        let program = match root.as_program_node() {
            Some(p) => p,
            None => return,
        };
        let loc = program.location();
        self.table
            .push_scope(ScopeKind::TopLevel, loc.start_offset(), loc.end_offset());
        self.fire_after_entering_scope();

        for stmt in program.statements().body().iter() {
            self.visit(&stmt);
        }

        self.leave_scope();
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    // ── Hook dispatch ──────────────────────────────────────────────────

    fn fire_after_entering_scope(&mut self) {
        let scope = self.table.current_scope();
        for rc in self.consumers {
            rc.consumer.after_entering_scope(
                scope,
                &self.table,
                self.source,
                rc.config,
                &mut self.diagnostics,
            );
        }
    }

    fn fire_before_leaving_scope(&mut self) {
        // Sync branch contexts to the table so consumers can access them.
        self.table.branch_contexts.clone_from(&self.branch_contexts);
        let scope = self.table.current_scope();
        for rc in self.consumers {
            rc.consumer.before_leaving_scope(
                scope,
                &self.table,
                self.source,
                rc.config,
                &mut self.diagnostics,
            );
        }
    }

    fn fire_after_leaving_scope(&mut self, scope: &super::Scope) {
        for rc in self.consumers {
            rc.consumer.after_leaving_scope(
                scope,
                &self.table,
                self.source,
                rc.config,
                &mut self.diagnostics,
            );
        }
    }

    // ── Scope management ───────────────────────────────────────────────

    fn enter_scope(&mut self, kind: ScopeKind, start: usize, end: usize) {
        self.table.push_scope(kind, start, end);
        self.fire_after_entering_scope();
    }

    fn leave_scope(&mut self) {
        self.fire_before_leaving_scope();
        let scope = self.table.pop_scope();
        self.fire_after_leaving_scope(&scope);
    }

    // ── Variable declaration with hooks ─────────────────────────────────

    fn declare_variable(&mut self, name: Vec<u8>, offset: usize, kind: DeclarationKind) {
        let temp_var =
            super::Variable::new(name.clone(), offset, kind, self.table.current_scope_index());
        for rc in self.consumers {
            rc.consumer.before_declaring_variable(
                &temp_var,
                &self.table,
                self.source,
                rc.config,
                &mut self.diagnostics,
            );
        }

        let created = self.table.declare_variable(name.clone(), offset, kind);
        if created {
            if let Some(var) = self.table.current_scope().variables.get(&name) {
                for rc in self.consumers {
                    rc.consumer.after_declaring_variable(
                        var,
                        &self.table,
                        self.source,
                        rc.config,
                        &mut self.diagnostics,
                    );
                }
            }
        }
    }

    // ── Parameter declaration ──────────────────────────────────────────

    fn declare_parameters(&mut self, params: &ruby_prism::ParametersNode<'_>) {
        for param in params.requireds().iter() {
            if let Some(rp) = param.as_required_parameter_node() {
                self.declare_variable(
                    rp.name().as_slice().to_vec(),
                    rp.location().start_offset(),
                    DeclarationKind::RequiredArg,
                );
            } else if let Some(mt) = param.as_multi_target_node() {
                self.declare_multi_target_params(&mt);
            }
        }
        for param in params.optionals().iter() {
            if let Some(op) = param.as_optional_parameter_node() {
                self.declare_variable(
                    op.name().as_slice().to_vec(),
                    op.location().start_offset(),
                    DeclarationKind::OptionalArg,
                );
                self.visit(&op.value());
            }
        }
        if let Some(rest) = params.rest() {
            if let Some(rp) = rest.as_rest_parameter_node() {
                if let Some(name) = rp.name() {
                    let offset = rp
                        .name_loc()
                        .map_or(rp.location().start_offset(), |loc| loc.start_offset());
                    self.declare_variable(
                        name.as_slice().to_vec(),
                        offset,
                        DeclarationKind::RestArg,
                    );
                }
            }
        }
        for param in params.posts().iter() {
            if let Some(rp) = param.as_required_parameter_node() {
                self.declare_variable(
                    rp.name().as_slice().to_vec(),
                    rp.location().start_offset(),
                    DeclarationKind::RequiredArg,
                );
            } else if let Some(mt) = param.as_multi_target_node() {
                self.declare_multi_target_params(&mt);
            }
        }
        for param in params.keywords().iter() {
            if let Some(kp) = param.as_required_keyword_parameter_node() {
                let mut name = kp.name().as_slice().to_vec();
                if name.last() == Some(&b':') {
                    name.pop();
                }
                self.declare_variable(
                    name,
                    kp.location().start_offset(),
                    DeclarationKind::KeywordArg,
                );
            } else if let Some(kp) = param.as_optional_keyword_parameter_node() {
                let mut name = kp.name().as_slice().to_vec();
                if name.last() == Some(&b':') {
                    name.pop();
                }
                self.declare_variable(
                    name,
                    kp.location().start_offset(),
                    DeclarationKind::OptionalKeywordArg,
                );
                self.visit(&kp.value());
            }
        }
        if let Some(kw_rest) = params.keyword_rest() {
            if let Some(krp) = kw_rest.as_keyword_rest_parameter_node() {
                if let Some(name) = krp.name() {
                    let offset = krp
                        .name_loc()
                        .map_or(krp.location().start_offset(), |loc| loc.start_offset());
                    self.declare_variable(
                        name.as_slice().to_vec(),
                        offset,
                        DeclarationKind::KeywordRestArg,
                    );
                }
            }
        }
        if let Some(block) = params.block() {
            if let Some(name) = block.name() {
                let offset = block
                    .name_loc()
                    .map_or(block.location().start_offset(), |loc| loc.start_offset());
                self.declare_variable(name.as_slice().to_vec(), offset, DeclarationKind::BlockArg);
            }
        }
    }

    fn declare_multi_target_params(&mut self, mt: &ruby_prism::MultiTargetNode<'_>) {
        for target in mt.lefts().iter() {
            if let Some(rp) = target.as_required_parameter_node() {
                self.declare_variable(
                    rp.name().as_slice().to_vec(),
                    rp.location().start_offset(),
                    DeclarationKind::RequiredArg,
                );
            } else if let Some(inner) = target.as_multi_target_node() {
                self.declare_multi_target_params(&inner);
            }
        }
        if let Some(rest) = mt.rest() {
            if let Some(splat) = rest.as_splat_node() {
                if let Some(expr) = splat.expression() {
                    if let Some(rp) = expr.as_required_parameter_node() {
                        self.declare_variable(
                            rp.name().as_slice().to_vec(),
                            rp.location().start_offset(),
                            DeclarationKind::RestArg,
                        );
                    }
                }
            }
        }
        for target in mt.rights().iter() {
            if let Some(rp) = target.as_required_parameter_node() {
                self.declare_variable(
                    rp.name().as_slice().to_vec(),
                    rp.location().start_offset(),
                    DeclarationKind::RequiredArg,
                );
            } else if let Some(inner) = target.as_multi_target_node() {
                self.declare_multi_target_params(&inner);
            }
        }
    }

    fn declare_block_parameters(&mut self, bp: &ruby_prism::BlockParametersNode<'_>) {
        if let Some(params) = bp.parameters() {
            self.declare_parameters(&params);
        }
        for local in bp.locals().iter() {
            if let Some(blv) = local.as_block_local_variable_node() {
                self.declare_variable(
                    blv.name().as_slice().to_vec(),
                    blv.location().start_offset(),
                    DeclarationKind::ShadowArg,
                );
            }
        }
    }

    fn declare_and_assign_for_targets(&mut self, node: &ruby_prism::Node<'_>) {
        struct TargetCollector {
            targets: Vec<(Vec<u8>, usize)>,
        }

        impl<'pr> ruby_prism::Visit<'pr> for TargetCollector {
            fn visit_local_variable_target_node(
                &mut self,
                node: &ruby_prism::LocalVariableTargetNode<'pr>,
            ) {
                self.targets.push((
                    node.name().as_slice().to_vec(),
                    node.location().start_offset(),
                ));
            }

            fn visit_def_node(&mut self, _: &ruby_prism::DefNode<'_>) {}
            fn visit_class_node(&mut self, _: &ruby_prism::ClassNode<'_>) {}
            fn visit_module_node(&mut self, _: &ruby_prism::ModuleNode<'_>) {}
        }

        let mut collector = TargetCollector {
            targets: Vec::new(),
        };
        collector.visit(node);

        for (name, offset) in collector.targets {
            if !self.table.variable_exists(&name) {
                self.declare_variable(name.clone(), offset, DeclarationKind::ForIndex);
            }

            let mut assignment = Assignment::new(offset, AssignmentKind::For);
            assignment.in_branch = self.branch_depth > 0;
            assignment.shadowing_in_branch = self.current_shadowing_in_branch();
            assignment.branch_id = self.current_branch_id();
            assignment.branch_path = self.current_branch_path();
            self.table.assign_to_variable(&name, assignment);
        }
    }
}

// ── Prism Visitor ──────────────────────────────────────────────────────

impl<'pr> Visit<'pr> for Engine<'_> {
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        let name = node.name().as_slice().to_vec();
        let offset = node.location().start_offset();
        if !self.table.variable_exists(&name) {
            self.declare_variable(name.clone(), offset, DeclarationKind::Assignment);
        }

        // Count explicit references before RHS to detect self-references.
        // Only explicit references count (not implicit ones from super/binding)
        // because RuboCop's `uses_var?` only matches `(lvar %)`.
        let explicit_refs_before = self
            .table
            .find_variable(&name)
            .map_or(0, |v| v.references.iter().filter(|r| r.explicit).count());

        self.visit(&node.value());

        let explicit_refs_after = self
            .table
            .find_variable(&name)
            .map_or(0, |v| v.references.iter().filter(|r| r.explicit).count());
        let rhs_refs_var = explicit_refs_after > explicit_refs_before;

        let seq = self.next_sequence();
        let mut assign = Assignment::new(offset, AssignmentKind::Simple);
        assign.sequence = seq;
        assign.rhs_references_var = rhs_refs_var;
        assign.in_branch = self.branch_depth > 0;
        assign.shadowing_in_branch = self.current_shadowing_in_branch();
        assign.branch_id = self.current_branch_id();
        assign.branch_path = self.current_branch_path();
        let val = node.value();
        assign.value_range = Some((val.location().start_offset(), val.location().end_offset()));
        self.table.assign_to_variable(&name, assign);
    }

    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        let scope_index = self.table.current_scope_index();
        let seq = self.next_sequence();
        let mut reference = Reference::new(node.location().start_offset(), scope_index);
        reference.sequence = seq;
        reference.branch_id = self.current_branch_id();
        reference.branch_path = self.current_branch_path();
        self.table
            .reference_variable(node.name().as_slice(), reference);
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        let name = node.name().as_slice().to_vec();
        let offset = node.location().start_offset();
        if !self.table.variable_exists(&name) {
            self.declare_variable(name.clone(), offset, DeclarationKind::Assignment);
        }
        let si = self.table.current_scope_index();
        let seq = self.next_sequence();
        let mut r = Reference::new(offset, si);
        r.sequence = seq;
        r.branch_id = self.current_branch_id();
        r.branch_path = self.current_branch_path();
        self.table.reference_variable(&name, r);
        self.visit(&node.value());
        let seq = self.next_sequence();
        let mut a = Assignment::new(offset, AssignmentKind::Operator);
        a.sequence = seq;
        a.rhs_references_var = true; // operator-writes always read the var
        a.in_branch = self.branch_depth > 0;
        a.shadowing_in_branch = self.current_shadowing_in_branch();
        a.branch_id = self.current_branch_id();
        a.branch_path = self.current_branch_path();
        self.table.assign_to_variable(&name, a);
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        let name = node.name().as_slice().to_vec();
        let offset = node.location().start_offset();
        if !self.table.variable_exists(&name) {
            self.declare_variable(name.clone(), offset, DeclarationKind::Assignment);
        }
        let si = self.table.current_scope_index();
        let seq = self.next_sequence();
        let mut r = Reference::new(offset, si);
        r.sequence = seq;
        r.branch_id = self.current_branch_id();
        r.branch_path = self.current_branch_path();
        self.table.reference_variable(&name, r);
        self.visit(&node.value());
        let seq = self.next_sequence();
        let mut a = Assignment::new(offset, AssignmentKind::LogicalOr);
        a.sequence = seq;
        a.rhs_references_var = true;
        a.in_branch = self.branch_depth > 0;
        a.shadowing_in_branch = self.current_shadowing_in_branch();
        a.branch_id = self.current_branch_id();
        a.branch_path = self.current_branch_path();
        self.table.assign_to_variable(&name, a);
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        let name = node.name().as_slice().to_vec();
        let offset = node.location().start_offset();
        if !self.table.variable_exists(&name) {
            self.declare_variable(name.clone(), offset, DeclarationKind::Assignment);
        }
        let si = self.table.current_scope_index();
        let seq = self.next_sequence();
        let mut r = Reference::new(offset, si);
        r.sequence = seq;
        r.branch_id = self.current_branch_id();
        r.branch_path = self.current_branch_path();
        self.table.reference_variable(&name, r);
        self.visit(&node.value());
        let seq = self.next_sequence();
        let mut a = Assignment::new(offset, AssignmentKind::LogicalAnd);
        a.sequence = seq;
        a.rhs_references_var = true;
        a.in_branch = self.branch_depth > 0;
        a.shadowing_in_branch = self.current_shadowing_in_branch();
        a.branch_id = self.current_branch_id();
        a.branch_path = self.current_branch_path();
        self.table.assign_to_variable(&name, a);
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        // Collect target names before visiting the RHS so we can detect self-refs
        let mut target_names: Vec<Vec<u8>> = Vec::new();
        for target in node.lefts().iter() {
            if let Some(t) = target.as_local_variable_target_node() {
                target_names.push(t.name().as_slice().to_vec());
            }
        }
        if let Some(rest) = node.rest() {
            if let Some(splat) = rest.as_splat_node() {
                if let Some(expr) = splat.expression() {
                    if let Some(t) = expr.as_local_variable_target_node() {
                        target_names.push(t.name().as_slice().to_vec());
                    }
                }
            }
        }
        for target in node.rights().iter() {
            if let Some(t) = target.as_local_variable_target_node() {
                target_names.push(t.name().as_slice().to_vec());
            }
        }

        // Bare `super` (ForwardingSuperNode) implicitly forwards all method
        // arguments, so any argument target in a `a, b = super` multi-write
        // is effectively "used" on the RHS.
        let rhs_is_forwarding_super = node.value().as_forwarding_super_node().is_some();

        // Snapshot explicit reference counts before RHS
        let refs_before: Vec<(Vec<u8>, usize)> = target_names
            .iter()
            .map(|name| {
                let count = self
                    .table
                    .find_variable(name)
                    .map_or(0, |v| v.references.iter().filter(|r| r.explicit).count());
                (name.clone(), count)
            })
            .collect();

        self.visit(&node.value());

        // Check which targets gained explicit references from the RHS.
        // If the RHS is bare `super`, treat all argument variables as referenced.
        let rhs_refs: Vec<(Vec<u8>, bool)> = refs_before
            .iter()
            .map(|(name, before)| {
                let explicitly_ref = {
                    let after = self
                        .table
                        .find_variable(name)
                        .map_or(0, |v| v.references.iter().filter(|r| r.explicit).count());
                    after > *before
                };
                let super_ref = rhs_is_forwarding_super
                    && self
                        .table
                        .find_variable(name)
                        .is_some_and(|v| v.is_argument());
                (name.clone(), explicitly_ref || super_ref)
            })
            .collect();

        let in_branch = self.branch_depth > 0;
        let shadowing_in_branch = self.current_shadowing_in_branch();
        let branch_id = self.current_branch_id();
        let branch_path = self.current_branch_path();
        let seq = self.next_sequence();

        for target in node.lefts().iter() {
            if let Some(t) = target.as_local_variable_target_node() {
                let name = t.name().as_slice().to_vec();
                let offset = t.location().start_offset();
                if !self.table.variable_exists(&name) {
                    self.declare_variable(name.clone(), offset, DeclarationKind::Assignment);
                }
                let rhs_refs_var = rhs_refs
                    .iter()
                    .find(|(n, _)| n == &name)
                    .is_some_and(|(_, r)| *r);
                let mut a = Assignment::new(offset, AssignmentKind::Multiple);
                a.in_branch = in_branch;
                a.shadowing_in_branch = shadowing_in_branch;
                a.branch_id = branch_id;
                a.branch_path = branch_path.clone();
                a.sequence = seq;
                a.rhs_references_var = rhs_refs_var;
                self.table.assign_to_variable(&name, a);
            } else {
                self.visit(&target);
            }
        }
        if let Some(rest) = node.rest() {
            if let Some(splat) = rest.as_splat_node() {
                if let Some(expr) = splat.expression() {
                    if let Some(t) = expr.as_local_variable_target_node() {
                        let name = t.name().as_slice().to_vec();
                        let offset = t.location().start_offset();
                        if !self.table.variable_exists(&name) {
                            self.declare_variable(
                                name.clone(),
                                offset,
                                DeclarationKind::Assignment,
                            );
                        }
                        let rhs_refs_var = rhs_refs
                            .iter()
                            .find(|(n, _)| n == &name)
                            .is_some_and(|(_, r)| *r);
                        let mut a = Assignment::new(offset, AssignmentKind::Rest);
                        a.in_branch = in_branch;
                        a.shadowing_in_branch = shadowing_in_branch;
                        a.branch_id = branch_id;
                        a.branch_path = branch_path.clone();
                        a.sequence = seq;
                        a.rhs_references_var = rhs_refs_var;
                        self.table.assign_to_variable(&name, a);
                    }
                }
            } else {
                self.visit(&rest);
            }
        }
        for target in node.rights().iter() {
            if let Some(t) = target.as_local_variable_target_node() {
                let name = t.name().as_slice().to_vec();
                let offset = t.location().start_offset();
                if !self.table.variable_exists(&name) {
                    self.declare_variable(name.clone(), offset, DeclarationKind::Assignment);
                }
                let rhs_refs_var = rhs_refs
                    .iter()
                    .find(|(n, _)| n == &name)
                    .is_some_and(|(_, r)| *r);
                let mut a = Assignment::new(offset, AssignmentKind::Multiple);
                a.in_branch = in_branch;
                a.shadowing_in_branch = shadowing_in_branch;
                a.branch_id = branch_id;
                a.branch_path = branch_path.clone();
                a.sequence = seq;
                a.rhs_references_var = rhs_refs_var;
                self.table.assign_to_variable(&name, a);
            } else {
                self.visit(&target);
            }
        }
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        if let Some(recv) = node.receiver() {
            self.visit(&recv);
        }
        let kind = if node.receiver().is_some() {
            ScopeKind::Defs
        } else {
            ScopeKind::Def
        };
        let loc = node.location();
        let saved_depth = self.branch_depth;
        let saved_stack = std::mem::take(&mut self.branch_stack);
        self.branch_depth = 0;
        self.enter_scope(kind, loc.start_offset(), loc.end_offset());
        if let Some(params) = node.parameters() {
            self.declare_parameters(&params);
        }
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.leave_scope();
        self.branch_depth = saved_depth;
        self.branch_stack = saved_stack;
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        let loc = node.location();
        let body_empty = node.body().is_none();
        // Save and reset branch_depth/branch_stack: block body starts a fresh
        // scope. Assignments to outer variables are marked captured_by_block
        // by the variable table, which the cop uses as a conditional indicator.
        // Clearing branch_stack ensures outer branches (e.g. a begin/ensure
        // body wrapping the block) do not bleed into in-block branch checks.
        let saved_depth = self.branch_depth;
        let saved_stack = std::mem::take(&mut self.branch_stack);
        self.branch_depth = 0;
        self.enter_scope(ScopeKind::Block, loc.start_offset(), loc.end_offset());
        self.table.current_scope_mut().body_empty = body_empty;
        if let Some(params) = node.parameters() {
            if let Some(bp) = params.as_block_parameters_node() {
                self.declare_block_parameters(&bp);
            }
        }
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.leave_scope();
        self.branch_depth = saved_depth;
        self.branch_stack = saved_stack;
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        let loc = node.location();
        let body_empty = node.body().is_none();
        let saved_depth = self.branch_depth;
        let saved_stack = std::mem::take(&mut self.branch_stack);
        self.branch_depth = 0;
        self.enter_scope(ScopeKind::Block, loc.start_offset(), loc.end_offset());
        self.table.current_scope_mut().body_empty = body_empty;
        if let Some(params) = node.parameters() {
            if let Some(bp) = params.as_block_parameters_node() {
                self.declare_block_parameters(&bp);
            }
        }
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.leave_scope();
        self.branch_depth = saved_depth;
        self.branch_stack = saved_stack;
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        if let Some(superclass) = node.superclass() {
            self.visit(&superclass);
        }
        let loc = node.location();
        self.enter_scope(ScopeKind::Class, loc.start_offset(), loc.end_offset());
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.leave_scope();
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        let loc = node.location();
        self.enter_scope(ScopeKind::Module, loc.start_offset(), loc.end_offset());
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.leave_scope();
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        self.visit(&node.expression());
        let loc = node.location();
        self.enter_scope(
            ScopeKind::SingletonClass,
            loc.start_offset(),
            loc.end_offset(),
        );
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.leave_scope();
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        let location = node.location();
        let parent_id = Self::branch_parent_id(&location);
        let is_modifier = node.end_keyword_loc().is_none() && node.if_keyword_loc().is_some();

        // Normal predicate assignments are unbranched because the condition
        // always runs before later reads. Only modifier form keeps a special
        // predicate context so the left-hand body can still see an earlier
        // assignment (`puts a if (a = 123)`).
        let pred_has_write = predicate_has_lvar_write(&node.predicate());
        if pred_has_write && is_modifier {
            self.branch_depth += 1;
            self.push_branch_with_flags(parent_id, 0, true, is_modifier, false, false, false);
        }
        self.visit(&node.predicate());
        if pred_has_write && is_modifier {
            self.pop_branch();
            self.branch_depth -= 1;
        }

        let body_child = if pred_has_write && is_modifier { 1 } else { 0 };
        self.branch_depth += 1;
        self.push_branch(parent_id, body_child, false);
        if let Some(stmts) = node.statements() {
            for stmt in stmts.body().iter() {
                self.visit(&stmt);
            }
        }
        self.pop_branch();
        self.branch_depth -= 1;
        if let Some(subsequent) = node.subsequent() {
            self.branch_depth += 1;
            self.push_branch(parent_id, body_child + 1, false);
            self.visit(&subsequent);
            self.pop_branch();
            self.branch_depth -= 1;
        }
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        let location = node.location();
        let parent_id = Self::branch_parent_id(&location);
        let is_modifier = node.end_keyword_loc().is_none();

        let pred_has_write = predicate_has_lvar_write(&node.predicate());
        if pred_has_write && is_modifier {
            self.branch_depth += 1;
            self.push_branch_with_flags(parent_id, 0, true, is_modifier, false, false, false);
        }
        self.visit(&node.predicate());
        if pred_has_write && is_modifier {
            self.pop_branch();
            self.branch_depth -= 1;
        }

        // Visit else clause FIRST, then unless body. The parser gem
        // represents `unless cond; A; else; B; end` as `(if cond B A)`,
        // so RuboCop's VF visits B (the else clause, as if-branch) before
        // A (the unless body, as else-branch). We must match this order
        // for `find_variable` to return the same declaration_offset as
        // RuboCop's VF.
        let body_child = if pred_has_write && is_modifier { 1 } else { 0 };
        self.branch_depth += 1;
        if let Some(else_clause) = node.else_clause() {
            self.push_branch(parent_id, body_child, false);
            if let Some(stmts) = else_clause.statements() {
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
            }
            self.pop_branch();
        }
        self.push_branch(parent_id, body_child + 1, false);
        if let Some(stmts) = node.statements() {
            for stmt in stmts.body().iter() {
                self.visit(&stmt);
            }
        }
        self.pop_branch();
        self.branch_depth -= 1;
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        let location = node.location();
        let parent_id = Self::branch_parent_id(&location);
        self.visit(&node.left());
        self.branch_depth += 1;
        self.push_branch_with_flags(parent_id, 1, false, false, false, false, true);
        self.visit(&node.right());
        self.pop_branch();
        self.branch_depth -= 1;
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        let location = node.location();
        let parent_id = Self::branch_parent_id(&location);
        self.visit(&node.left());
        self.branch_depth += 1;
        self.push_branch_with_flags(parent_id, 1, false, false, false, false, true);
        self.visit(&node.right());
        self.pop_branch();
        self.branch_depth -= 1;
    }

    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'pr>) {
        let location = node.location();
        let parent_id = Self::branch_parent_id(&location);
        // `case` targets are always evaluated before any clause runs, so
        // assignments inside them are unbranched for liveness purposes. Bump
        // branch_depth without a context so ShadowedArgument's
        // `current_shadowing_in_branch` still treats a predicate assignment
        // like `case value = super` as conditional.
        if let Some(pred) = node.predicate() {
            let pred_has_write = predicate_has_lvar_write(&pred);
            if pred_has_write {
                self.branch_depth += 1;
            }
            self.visit(&pred);
            if pred_has_write {
                self.branch_depth -= 1;
            }
        }
        self.branch_depth += 1;
        for (i, condition) in node.conditions().iter().enumerate() {
            self.push_branch(parent_id, i, false);
            self.visit(&condition);
            self.pop_branch();
        }
        if let Some(else_clause) = node.else_clause() {
            let else_idx = node.conditions().len();
            self.push_branch(parent_id, else_idx, false);
            if let Some(stmts) = else_clause.statements() {
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
            }
            self.pop_branch();
        }
        self.branch_depth -= 1;
    }

    fn visit_in_node(&mut self, node: &ruby_prism::InNode<'pr>) {
        // Declare and assign all pattern match variables before visiting the
        // pattern. Guard clauses like `in _ if _.blank?` are represented as
        // IfNode where the predicate (guard) references the variable before
        // the pattern target declares it. Pre-declaring and assigning ensures
        // the variable exists when the guard's LocalVariableReadNode is visited.
        // The default visitor traversal is safe because there is no generic
        // visit_local_variable_target_node handler to cause double assignments.
        declare_and_assign_pattern_targets(self, &node.pattern());
        ruby_prism::visit_in_node(self, node);
    }

    fn visit_match_required_node(&mut self, node: &ruby_prism::MatchRequiredNode<'pr>) {
        // `expr => pattern` creates local variables that remain visible in the
        // surrounding scope. RuboCop's `process_pattern_match_variable` only
        // declares these — it does not record an assignment — so cops that
        // iterate `variable.assignments` (e.g. RSpec/LeakyLocalVariable) stay
        // quiet on pattern-match-only bindings. Match that behavior.
        declare_pattern_targets(self, &node.pattern());
        ruby_prism::visit_match_required_node(self, node);
    }

    fn visit_case_match_node(&mut self, node: &ruby_prism::CaseMatchNode<'pr>) {
        let location = node.location();
        let parent_id = Self::branch_parent_id(&location);
        if let Some(pred) = node.predicate() {
            let pred_has_write = predicate_has_lvar_write(&pred);
            if pred_has_write {
                self.branch_depth += 1;
            }
            self.visit(&pred);
            if pred_has_write {
                self.branch_depth -= 1;
            }
        }
        self.branch_depth += 1;
        for (i, condition) in node.conditions().iter().enumerate() {
            self.push_branch(parent_id, i, false);
            self.visit(&condition);
            self.pop_branch();
        }
        if let Some(else_clause) = node.else_clause() {
            let else_idx = node.conditions().len();
            self.push_branch(parent_id, else_idx, false);
            if let Some(stmts) = else_clause.statements() {
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
            }
            self.pop_branch();
        }
        self.branch_depth -= 1;
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        let location = node.location();
        let parent_id = Self::branch_parent_id(&location);
        let is_post_condition = node.is_begin_modifier();

        if is_post_condition {
            // Post-condition loop (begin...end while): visit body first,
            // then condition. Matches RuboCop's VariableForce which processes
            // body before condition for while_post/until_post nodes.
            let body_child = 0;
            self.branch_depth += 1;
            self.push_branch(parent_id, body_child, false);
            if let Some(stmts) = node.statements() {
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
            }
            self.pop_branch();
            self.branch_depth -= 1;

            self.visit(&node.predicate());
        } else {
            // Pre-condition loop (while...end): visit condition first, then body.
            let pred_has_write = predicate_has_lvar_write(&node.predicate());
            let is_modifier = node.do_keyword_loc().is_none() && node.closing_loc().is_none();
            if pred_has_write && is_modifier {
                self.branch_depth += 1;
                self.push_branch_with_flags(parent_id, 0, true, is_modifier, false, false, false);
            }
            self.visit(&node.predicate());
            if pred_has_write && is_modifier {
                self.pop_branch();
                self.branch_depth -= 1;
            }

            let body_child = if pred_has_write && is_modifier { 1 } else { 0 };
            self.branch_depth += 1;
            self.push_branch(parent_id, body_child, false);
            if let Some(stmts) = node.statements() {
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
            }
            self.pop_branch();
            self.branch_depth -= 1;
        }
        let loc = node.location();
        self.mark_loop_back_edges(loc.start_offset(), loc.end_offset());
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        let location = node.location();
        let parent_id = Self::branch_parent_id(&location);
        let is_post_condition = node.is_begin_modifier();

        if is_post_condition {
            // Post-condition loop (begin...end until): visit body first,
            // then condition. Matches RuboCop's VariableForce.
            let body_child = 0;
            self.branch_depth += 1;
            self.push_branch(parent_id, body_child, false);
            if let Some(stmts) = node.statements() {
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
            }
            self.pop_branch();
            self.branch_depth -= 1;

            self.visit(&node.predicate());
        } else {
            // Pre-condition loop (until...end): visit condition first, then body.
            let pred_has_write = predicate_has_lvar_write(&node.predicate());
            let is_modifier = node.do_keyword_loc().is_none() && node.closing_loc().is_none();
            if pred_has_write && is_modifier {
                self.branch_depth += 1;
                self.push_branch_with_flags(parent_id, 0, true, is_modifier, false, false, false);
            }
            self.visit(&node.predicate());
            if pred_has_write && is_modifier {
                self.pop_branch();
                self.branch_depth -= 1;
            }

            let body_child = if pred_has_write && is_modifier { 1 } else { 0 };
            self.branch_depth += 1;
            self.push_branch(parent_id, body_child, false);
            if let Some(stmts) = node.statements() {
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
            }
            self.pop_branch();
            self.branch_depth -= 1;
        }
        let loc = node.location();
        self.mark_loop_back_edges(loc.start_offset(), loc.end_offset());
    }

    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        // Branch context is managed by the caller (`visit_begin_node`), which
        // visits each rescue clause under its own sibling branch context.
        // Visit children manually instead of delegating to the default visitor
        // so we can handle the exception capture variable explicitly.

        // Exception class references
        for exc in node.exceptions().iter() {
            self.visit(&exc);
        }

        // Exception capture: `rescue Error => e`
        if let Some(ref_node) = node.reference() {
            if let Some(t) = ref_node.as_local_variable_target_node() {
                let name = t.name().as_slice().to_vec();
                let offset = t.location().start_offset();
                if !self.table.variable_exists(&name) {
                    self.declare_variable(name.clone(), offset, DeclarationKind::Assignment);
                }
                let seq = self.next_sequence();
                let mut a = Assignment::new(offset, AssignmentKind::ExceptionCapture);
                a.sequence = seq;
                a.in_branch = self.branch_depth > 0;
                a.shadowing_in_branch = self.current_shadowing_in_branch();
                a.branch_id = self.current_branch_id();
                a.branch_path = self.current_branch_path();
                self.table.assign_to_variable(&name, a);
            }
        }

        // Rescue body statements
        if let Some(stmts) = node.statements() {
            for stmt in stmts.body().iter() {
                self.visit(&stmt);
            }
        }

        // If any rescue clause contains a `retry`, treat the entire rescue
        // as a loop — the retry causes the begin body to re-execute.
        if rescue_contains_retry(node) {
            let loc = node.location();
            self.mark_loop_back_edges(loc.start_offset(), loc.end_offset());
        }
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        // `begin ... rescue ... else ... end` behaves like RuboCop's rescue
        // branch model:
        // - the main body may jump into a rescue clause or stop before later
        //   writes, so it is not exclusive with sibling rescue/else reads
        // - each rescue clause is its own exclusive sibling branch
        // - the else clause is its own sibling branch
        if let Some(first_rescue) = node.rescue_clause() {
            let location = node.location();
            let parent_id = Self::branch_parent_id(&location);

            if let Some(stmts) = node.statements() {
                self.branch_depth += 1;
                self.push_branch_with_flags(parent_id, 0, false, false, true, true, false);
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
                self.pop_branch();
                self.branch_depth -= 1;
            }

            // Each rescue clause is its own exclusive sibling branch.
            let mut clause_index = 1;
            let mut current = Some(first_rescue);
            while let Some(rescue_clause) = current {
                let next = rescue_clause.subsequent();
                self.branch_depth += 1;
                self.push_branch(parent_id, clause_index, false);
                self.visit_rescue_node(&rescue_clause);
                self.pop_branch();
                self.branch_depth -= 1;
                clause_index += 1;
                current = next;
            }

            if let Some(else_clause) = node.else_clause() {
                self.branch_depth += 1;
                self.push_branch(parent_id, clause_index, false);
                if let Some(stmts) = else_clause.statements() {
                    for stmt in stmts.body().iter() {
                        self.visit(&stmt);
                    }
                }
                self.pop_branch();
                self.branch_depth -= 1;
            }
        } else if let Some(stmts) = node.statements() {
            if node.ensure_clause().is_some() {
                let location = node.location();
                let parent_id = Self::branch_parent_id(&location);
                self.branch_depth += 1;
                self.push_branch_with_flags(parent_id, 0, false, false, true, true, false);
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
                self.pop_branch();
                self.branch_depth -= 1;
            } else {
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
            }
        }

        // Ensure clause — NOT branched (always executes)
        if let Some(ensure_clause) = node.ensure_clause() {
            if let Some(stmts) = ensure_clause.statements() {
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
            }
        }

        // If begin..rescue contains a retry, treat the entire begin block
        // as a loop for back-edge purposes.
        if let Some(rescue_clause) = node.rescue_clause() {
            if rescue_contains_retry(&rescue_clause) {
                let loc = node.location();
                self.mark_loop_back_edges(loc.start_offset(), loc.end_offset());
            }
        }
    }

    fn visit_match_write_node(&mut self, node: &ruby_prism::MatchWriteNode<'pr>) {
        // Named capture regex: `/(?<x>\w+)/ =~ str`
        // Visit the call (which contains the regex and the RHS) first.
        self.visit_call_node(&node.call());

        // Declare each captured variable. The declaration offset points at the
        // regex (the receiver of the =~ call), matching RuboCop's behavior.
        let call = node.call();
        let regex_offset = call.receiver().map_or(call.location().start_offset(), |r| {
            r.location().start_offset()
        });

        let in_branch = self.branch_depth > 0;
        let shadowing_in_branch = self.current_shadowing_in_branch();
        let branch_id = self.current_branch_id();
        let branch_path = self.current_branch_path();
        let seq = self.next_sequence();

        for target in node.targets().iter() {
            if let Some(t) = target.as_local_variable_target_node() {
                let name = t.name().as_slice().to_vec();
                if !self.table.variable_exists(&name) {
                    self.declare_variable(
                        name.clone(),
                        regex_offset,
                        DeclarationKind::RegexpCapture,
                    );
                }
                let mut a = Assignment::new(regex_offset, AssignmentKind::RegexpCapture);
                a.in_branch = in_branch;
                a.shadowing_in_branch = shadowing_in_branch;
                a.branch_id = branch_id;
                a.branch_path = branch_path.clone();
                a.sequence = seq;
                self.table.assign_to_variable(&name, a);
            }
        }
    }

    fn visit_for_node(&mut self, node: &ruby_prism::ForNode<'pr>) {
        self.visit(&node.collection());
        let index = node.index();
        self.declare_and_assign_for_targets(&index);
        if let Some(stmts) = node.statements() {
            for stmt in stmts.body().iter() {
                self.visit(&stmt);
            }
        }
        let loc = node.location();
        self.mark_loop_back_edges(loc.start_offset(), loc.end_offset());
    }

    fn visit_forwarding_super_node(&mut self, node: &ruby_prism::ForwardingSuperNode<'pr>) {
        let offset = node.location().start_offset();
        let si = self.table.current_scope_index();
        let branch_id = self.current_branch_id();
        let branch_path = self.current_branch_path();
        // Bare `super` forwards the enclosing method's arguments, not block params.
        // Only mark method-scope arguments as implicitly referenced.
        for var in self.table.enclosing_method_arguments_mut() {
            let mut reference = Reference::implicit(offset, si);
            reference.branch_id = branch_id;
            reference.branch_path = branch_path.clone();
            var.reference(reference);
        }
        // Visit the block child so that `super do |x| ... end` declares
        // block params and visits the block body.
        if let Some(block) = node.block() {
            self.visit_block_node(&block);
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        // Detect bare `binding` calls (Kernel#binding) which capture all local vars.
        // RuboCop's Parser AST treats `binding(&block)` as having arguments (the
        // block-pass is a child of the send node), so it does NOT count as bare
        // `binding`. In Prism, block-pass is separate from arguments, so we must
        // also check that the call's block is not a BlockArgumentNode.
        if node.name().as_slice() == b"binding"
            && node.arguments().is_none()
            && node
                .block()
                .is_none_or(|b| b.as_block_argument_node().is_none())
        {
            let offset = node.location().start_offset();
            let si = self.table.current_scope_index();
            let branch_id = self.current_branch_id();
            let branch_path = self.current_branch_path();
            for var in self.table.accessible_variables_mut() {
                let mut reference = Reference::implicit(offset, si);
                reference.branch_id = branch_id;
                reference.branch_path = branch_path.clone();
                var.reference(reference);
            }
        }
        if let Some(recv) = node.receiver() {
            self.visit(&recv);
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
}

/// Check if a predicate expression contains a local variable write.
/// Used to detect modifier-if patterns like `puts a if (a = 123)`.
fn predicate_has_lvar_write(node: &ruby_prism::Node<'_>) -> bool {
    struct LvarWriteDetector {
        found: bool,
    }
    impl<'pr> ruby_prism::Visit<'pr> for LvarWriteDetector {
        fn visit_local_variable_write_node(
            &mut self,
            _node: &ruby_prism::LocalVariableWriteNode<'pr>,
        ) {
            self.found = true;
        }
        // Don't recurse into nested scopes
        fn visit_def_node(&mut self, _node: &ruby_prism::DefNode<'pr>) {}
        fn visit_class_node(&mut self, _node: &ruby_prism::ClassNode<'pr>) {}
        fn visit_module_node(&mut self, _node: &ruby_prism::ModuleNode<'pr>) {}
    }
    let mut detector = LvarWriteDetector { found: false };
    detector.visit(node);
    detector.found
}

/// Declare and assign all `LocalVariableTargetNode` variables found in a
/// pattern match node. This ensures guard clause references (`in _ if _.blank?`)
/// can find the variable before the pattern target node is visited. Without the
/// generic `visit_local_variable_target_node` handler, this function must also
/// create assignments (not just declarations) for pattern match variables.
fn collect_pattern_targets(node: &ruby_prism::Node<'_>) -> Vec<(Vec<u8>, usize)> {
    struct TargetCollector {
        targets: Vec<(Vec<u8>, usize)>,
    }
    impl<'pr> ruby_prism::Visit<'pr> for TargetCollector {
        fn visit_local_variable_target_node(
            &mut self,
            node: &ruby_prism::LocalVariableTargetNode<'pr>,
        ) {
            self.targets.push((
                node.name().as_slice().to_vec(),
                node.location().start_offset(),
            ));
        }
        fn visit_def_node(&mut self, _: &ruby_prism::DefNode<'_>) {}
        fn visit_class_node(&mut self, _: &ruby_prism::ClassNode<'_>) {}
        fn visit_module_node(&mut self, _: &ruby_prism::ModuleNode<'_>) {}
    }
    let mut collector = TargetCollector {
        targets: Vec::new(),
    };
    collector.visit(node);
    collector.targets
}

fn declare_and_assign_pattern_targets(engine: &mut Engine<'_>, node: &ruby_prism::Node<'_>) {
    for (name, offset) in collect_pattern_targets(node) {
        if !engine.table.variable_exists(&name) {
            engine.declare_variable(name.clone(), offset, DeclarationKind::PatternMatch);
        }
        let seq = engine.next_sequence();
        let mut a = Assignment::new(offset, AssignmentKind::Simple);
        a.sequence = seq;
        a.in_branch = engine.branch_depth > 0;
        a.shadowing_in_branch = engine.current_shadowing_in_branch();
        a.branch_id = engine.current_branch_id();
        a.branch_path = engine.current_branch_path();
        engine.table.assign_to_variable(&name, a);
    }
}

fn declare_pattern_targets(engine: &mut Engine<'_>, node: &ruby_prism::Node<'_>) {
    for (name, offset) in collect_pattern_targets(node) {
        if !engine.table.variable_exists(&name) {
            engine.declare_variable(name.clone(), offset, DeclarationKind::PatternMatch);
        }
    }
}

/// Check if a rescue node (or its chained subsequent rescue clauses)
/// contains a `retry` statement anywhere in its descendants.
fn rescue_contains_retry(node: &ruby_prism::RescueNode<'_>) -> bool {
    struct RetryDetector {
        found: bool,
    }
    impl<'pr> ruby_prism::Visit<'pr> for RetryDetector {
        fn visit_retry_node(&mut self, _node: &ruby_prism::RetryNode<'pr>) {
            self.found = true;
        }
        // Don't recurse into new scopes
        fn visit_def_node(&mut self, _node: &ruby_prism::DefNode<'pr>) {}
        fn visit_class_node(&mut self, _node: &ruby_prism::ClassNode<'pr>) {}
        fn visit_module_node(&mut self, _node: &ruby_prism::ModuleNode<'pr>) {}
    }

    let mut detector = RetryDetector { found: false };
    // Check the rescue clause's body
    if let Some(stmts) = node.statements() {
        detector.visit(&stmts.as_node());
    }
    if detector.found {
        return true;
    }
    // Check subsequent rescue clauses
    if let Some(subsequent) = node.subsequent() {
        return rescue_contains_retry(&subsequent);
    }
    false
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;
    use crate::cop::variable_force::variable::DeclarationKind;

    /// A test consumer that collects scope/variable data during hooks.
    struct TestConsumer {
        /// Variables seen in before_leaving_scope, keyed by scope kind.
        /// Each entry: (scope_kind, {var_name: (assignments_count, references_count, declaration_kind)})
        scopes: RefCell<Vec<ScopeSnapshot>>,
        /// Variables seen in before_declaring_variable (for shadowing tests).
        declarations: RefCell<Vec<(Vec<u8>, bool)>>, // (name, outer_exists)
    }

    #[derive(Debug)]
    struct ScopeSnapshot {
        kind: ScopeKind,
        vars: HashMap<String, VarSnapshot>,
    }

    #[derive(Debug)]
    struct VarSnapshot {
        decl_kind: DeclarationKind,
        num_assignments: usize,
        num_references: usize,
        captured_by_block: bool,
        used: bool,
        has_implicit_ref: bool,
        /// Whether any assignment has rhs_references_var set.
        has_self_ref_assignment: bool,
        /// Per-assignment details for branch/liveness testing.
        assignments: Vec<AssignSnapshot>,
    }

    #[derive(Debug)]
    struct AssignSnapshot {
        referenced: bool,
        reassigned: bool,
        branch_id: Option<usize>,
        kind: crate::cop::variable_force::assignment::AssignmentKind,
    }

    impl TestConsumer {
        fn new() -> Self {
            Self {
                scopes: RefCell::new(Vec::new()),
                declarations: RefCell::new(Vec::new()),
            }
        }
    }

    impl VariableForceConsumer for TestConsumer {
        fn before_leaving_scope(
            &self,
            scope: &super::super::Scope,
            _table: &VariableTable,
            _source: &SourceFile,
            _config: &CopConfig,
            _diagnostics: &mut Vec<Diagnostic>,
        ) {
            let mut vars = HashMap::new();
            for (name, var) in &scope.variables {
                vars.insert(
                    String::from_utf8_lossy(name).to_string(),
                    VarSnapshot {
                        decl_kind: var.declaration_kind,
                        num_assignments: var.assignments.len(),
                        num_references: var.references.len(),
                        captured_by_block: var.captured_by_block,
                        used: var.used(),
                        has_implicit_ref: var.references.iter().any(|r| !r.explicit),
                        has_self_ref_assignment: var
                            .assignments
                            .iter()
                            .any(|a| a.rhs_references_var),
                        assignments: var
                            .assignments
                            .iter()
                            .map(|a| AssignSnapshot {
                                referenced: a.referenced,
                                reassigned: a.reassigned,
                                branch_id: a.branch_id,
                                kind: a.kind,
                            })
                            .collect(),
                    },
                );
            }
            self.scopes.borrow_mut().push(ScopeSnapshot {
                kind: scope.kind,
                vars,
            });
        }

        fn before_declaring_variable(
            &self,
            variable: &super::super::Variable,
            table: &VariableTable,
            _source: &SourceFile,
            _config: &CopConfig,
            _diagnostics: &mut Vec<Diagnostic>,
        ) {
            let outer_exists = table.find_variable(&variable.name).is_some();
            self.declarations
                .borrow_mut()
                .push((variable.name.clone(), outer_exists));
        }
    }

    // We need Send+Sync for the trait bounds
    unsafe impl Send for TestConsumer {}
    unsafe impl Sync for TestConsumer {}

    fn run_with_consumer(source: &str) -> (Vec<ScopeSnapshot>, Vec<(Vec<u8>, bool)>) {
        let sf = SourceFile::from_bytes("test.rb", source.as_bytes().to_vec());
        let pr = ruby_prism::parse(source.as_bytes());
        let consumer = TestConsumer::new();
        let config = CopConfig::default();
        let rc = vec![RegisteredConsumer {
            consumer: &consumer,
            config: &config,
        }];
        let mut engine = Engine::new(&sf, &rc);
        engine.run(&pr);
        let scopes = consumer.scopes.into_inner();
        let decls = consumer.declarations.into_inner();
        (scopes, decls)
    }

    fn run_engine(source: &str) -> Vec<ScopeSnapshot> {
        run_with_consumer(source).0
    }

    // ── Variable tracking tests ────────────────────────────────────────

    #[test]
    fn test_assignment_and_reference_tracked() {
        let scopes = run_engine("x = 1\nputs x\n");
        // TopLevel scope should have variable x
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].kind, ScopeKind::TopLevel);
        let x = &scopes[0].vars["x"];
        assert_eq!(x.num_assignments, 1);
        assert_eq!(x.num_references, 1);
        assert!(x.used);
    }

    #[test]
    fn test_unused_variable() {
        let scopes = run_engine("x = 1\n");
        let x = &scopes[0].vars["x"];
        assert_eq!(x.num_assignments, 1);
        assert_eq!(x.num_references, 0);
        assert!(!x.used);
    }

    #[test]
    fn test_multiple_assignments() {
        let scopes = run_engine("x = 1\nx = 2\nputs x\n");
        let x = &scopes[0].vars["x"];
        assert_eq!(x.num_assignments, 2);
        assert_eq!(x.num_references, 1);
    }

    #[test]
    fn test_self_referencing_assignment() {
        // x = x + 1 should create a reference BEFORE the second assignment
        let scopes = run_engine("x = 1\nx = x + 1\n");
        let x = &scopes[0].vars["x"];
        assert_eq!(x.num_assignments, 2);
        assert_eq!(x.num_references, 1); // x on RHS of second assignment
        assert!(x.has_self_ref_assignment); // second assignment references x on RHS
    }

    #[test]
    fn test_non_self_referencing_assignment() {
        let scopes = run_engine("x = 1\nx = 2\n");
        let x = &scopes[0].vars["x"];
        assert!(!x.has_self_ref_assignment); // x = 2 does NOT reference x
    }

    #[test]
    fn test_operator_write_always_self_refs() {
        let scopes = run_engine("x = 1\nx += 2\n");
        let x = &scopes[0].vars["x"];
        assert!(x.has_self_ref_assignment); // += always reads x
    }

    #[test]
    fn test_operator_assignment_creates_reference() {
        let scopes = run_engine("x = 1\nx += 2\n");
        let x = &scopes[0].vars["x"];
        assert_eq!(x.num_assignments, 2); // x = 1, x += 2
        assert_eq!(x.num_references, 1); // += reads x
        assert!(x.used);
    }

    #[test]
    fn test_or_write_creates_reference() {
        let scopes = run_engine("x = nil\nx ||= 1\n");
        let x = &scopes[0].vars["x"];
        assert_eq!(x.num_assignments, 2);
        assert_eq!(x.num_references, 1); // ||= reads x
    }

    #[test]
    fn test_and_write_creates_reference() {
        let scopes = run_engine("x = true\nx &&= false\n");
        let x = &scopes[0].vars["x"];
        assert_eq!(x.num_assignments, 2);
        assert_eq!(x.num_references, 1);
    }

    // ── Scope boundary tests ───────────────────────────────────────────

    #[test]
    fn test_def_is_hard_scope() {
        let scopes = run_engine("x = 1\ndef foo\n  y = 2\n  puts x\nend\n");
        // Should have 2 scopes: TopLevel and Def
        assert_eq!(scopes.len(), 2);

        // Def scope has y but NOT x (hard boundary)
        let def_scope = &scopes[0]; // inner scope popped first
        assert_eq!(def_scope.kind, ScopeKind::Def);
        assert!(def_scope.vars.contains_key("y"));
        assert!(!def_scope.vars.contains_key("x"));

        // TopLevel has x
        let top_scope = &scopes[1];
        assert_eq!(top_scope.kind, ScopeKind::TopLevel);
        assert!(top_scope.vars.contains_key("x"));
        // x is NOT referenced (the `puts x` inside def can't see it)
        assert_eq!(top_scope.vars["x"].num_references, 0);
    }

    #[test]
    fn test_block_captures_outer_variable() {
        let scopes = run_engine("x = 1\n[1].each { |i| puts x }\n");
        // Block scope and TopLevel scope
        assert_eq!(scopes.len(), 2);

        let block_scope = &scopes[0];
        assert_eq!(block_scope.kind, ScopeKind::Block);
        assert!(block_scope.vars.contains_key("i"));

        let top_scope = &scopes[1];
        assert!(top_scope.vars.contains_key("x"));
        // x IS referenced (block captures it) and captured_by_block
        assert_eq!(top_scope.vars["x"].num_references, 1);
        assert!(top_scope.vars["x"].captured_by_block);
    }

    #[test]
    fn test_class_is_hard_scope() {
        let scopes = run_engine("x = 1\nclass Foo\n  y = 2\nend\n");
        let class_scope = &scopes[0];
        assert_eq!(class_scope.kind, ScopeKind::Class);
        assert!(class_scope.vars.contains_key("y"));
        assert!(!class_scope.vars.contains_key("x"));
    }

    #[test]
    fn test_module_is_hard_scope() {
        let scopes = run_engine("x = 1\nmodule Foo\n  y = 2\nend\n");
        let mod_scope = &scopes[0];
        assert_eq!(mod_scope.kind, ScopeKind::Module);
        assert!(mod_scope.vars.contains_key("y"));
    }

    #[test]
    fn test_class_superclass_in_outer_scope() {
        let scopes = run_engine("base = Object\nclass Foo < base\n  x = 1\nend\n");
        // `base` should be referenced in the TopLevel scope (outer), not the Class scope
        let top = scopes
            .iter()
            .find(|s| s.kind == ScopeKind::TopLevel)
            .unwrap();
        assert!(top.vars["base"].num_references > 0);
    }

    #[test]
    fn test_singleton_class_receiver_in_outer_scope() {
        let scopes = run_engine("obj = Object.new\nclass << obj\n  x = 1\nend\n");
        let top = scopes
            .iter()
            .find(|s| s.kind == ScopeKind::TopLevel)
            .unwrap();
        assert!(top.vars["obj"].num_references > 0);
    }

    // ── Parameter declaration tests ────────────────────────────────────

    #[test]
    fn test_method_params_declared() {
        let scopes = run_engine("def foo(a, b = 1, *c, d:, e: 2, **f, &g)\nend\n");
        let def_scope = &scopes[0];
        assert_eq!(def_scope.kind, ScopeKind::Def);
        for name in &["a", "b", "c", "d", "e", "f", "g"] {
            assert!(def_scope.vars.contains_key(*name), "missing param: {name}");
        }
        assert_eq!(def_scope.vars["a"].decl_kind, DeclarationKind::RequiredArg);
        assert_eq!(def_scope.vars["b"].decl_kind, DeclarationKind::OptionalArg);
        assert_eq!(def_scope.vars["c"].decl_kind, DeclarationKind::RestArg);
        assert_eq!(def_scope.vars["d"].decl_kind, DeclarationKind::KeywordArg);
        assert_eq!(
            def_scope.vars["e"].decl_kind,
            DeclarationKind::OptionalKeywordArg
        );
        assert_eq!(
            def_scope.vars["f"].decl_kind,
            DeclarationKind::KeywordRestArg
        );
        assert_eq!(def_scope.vars["g"].decl_kind, DeclarationKind::BlockArg);
    }

    #[test]
    fn test_block_params_declared() {
        let scopes = run_engine("[1].each { |x, *y; local| }\n");
        let block_scope = &scopes[0];
        assert!(block_scope.vars.contains_key("x"));
        assert!(block_scope.vars.contains_key("y"));
        assert!(block_scope.vars.contains_key("local"));
        assert_eq!(
            block_scope.vars["local"].decl_kind,
            DeclarationKind::ShadowArg
        );
    }

    #[test]
    fn test_lambda_params_declared() {
        let scopes = run_engine("f = -> (x, y) { x + y }\n");
        let lambda_scope = &scopes[0];
        assert_eq!(lambda_scope.kind, ScopeKind::Block);
        assert!(lambda_scope.vars.contains_key("x"));
        assert!(lambda_scope.vars.contains_key("y"));
    }

    // ── Special node tests ─────────────────────────────────────────────

    #[test]
    fn test_binding_references_all_vars() {
        let scopes = run_engine("def foo(x)\n  y = 1\n  binding\nend\n");
        let def_scope = &scopes[0];
        // binding should reference both x and y
        assert!(def_scope.vars["x"].num_references > 0);
        assert!(def_scope.vars["y"].num_references > 0);
        // references from binding are implicit
        assert!(def_scope.vars["x"].has_implicit_ref);
    }

    #[test]
    fn test_forwarding_super_references_args() {
        let scopes = run_engine("def foo(x, y)\n  super\nend\n");
        let def_scope = &scopes[0];
        assert!(def_scope.vars["x"].num_references > 0);
        assert!(def_scope.vars["y"].num_references > 0);
        assert!(def_scope.vars["x"].has_implicit_ref);
    }

    #[test]
    fn test_forwarding_super_does_not_ref_locals() {
        let scopes = run_engine("def foo(x)\n  y = 1\n  super\nend\n");
        let def_scope = &scopes[0];
        assert!(def_scope.vars["x"].num_references > 0); // arg referenced
        assert_eq!(def_scope.vars["y"].num_references, 0); // local NOT referenced
    }

    #[test]
    fn test_forwarding_super_visits_block() {
        // `super do |x| puts x end` — the block child of ForwardingSuperNode
        // must be visited so that block params are declared in a new scope.
        let scopes = run_engine("def foo(a)\n  super do |x|\n    puts x\n  end\nend\n");
        // Should have at least 2 scopes: the def scope and the block scope
        assert!(scopes.len() >= 2);
        // The block scope should contain `x` as a block param
        let block_scope = scopes
            .iter()
            .find(|s| s.vars.contains_key("x"))
            .expect("block param x should be declared");
        assert_eq!(block_scope.kind, ScopeKind::Block);
        assert!(block_scope.vars["x"].used);
    }

    // ── Multi-write tests ──────────────────────────────────────────────

    #[test]
    fn test_multi_write() {
        let scopes = run_engine("a, b = 1, 2\nputs a\n");
        let top = &scopes[0];
        assert!(top.vars.contains_key("a"));
        assert!(top.vars.contains_key("b"));
        assert_eq!(top.vars["a"].num_assignments, 1);
        assert_eq!(top.vars["b"].num_assignments, 1);
        assert!(top.vars["a"].used);
        assert!(!top.vars["b"].used);
    }

    #[test]
    fn test_multi_write_with_splat() {
        let scopes = run_engine("a, *b = [1, 2, 3]\n");
        let top = &scopes[0];
        assert!(top.vars.contains_key("a"));
        assert!(top.vars.contains_key("b"));
    }

    // ── For loop tests ─────────────────────────────────────────────────

    #[test]
    fn test_for_loop_index_variable() {
        let scopes = run_engine("for x in [1, 2, 3]\n  puts x\nend\n");
        // for loop doesn't create a new scope — x is in TopLevel
        let top = &scopes[0];
        assert!(top.vars.contains_key("x"));
        assert_eq!(top.vars["x"].decl_kind, DeclarationKind::ForIndex);
        assert!(top.vars["x"].used);
    }

    // ── Nested scope tests ─────────────────────────────────────────────

    #[test]
    fn test_nested_blocks_capture_outer() {
        let scopes = run_engine("x = 1\n[1].each { |i| [2].each { |j| puts x } }\n");
        let top = scopes
            .iter()
            .find(|s| s.kind == ScopeKind::TopLevel)
            .unwrap();
        assert!(top.vars["x"].captured_by_block);
        assert!(top.vars["x"].used);
    }

    #[test]
    fn test_def_inside_block() {
        // def creates a hard boundary even inside a block
        let scopes = run_engine("x = 1\n[1].each { |i| def bar; y = x; end }\n");
        let top = scopes
            .iter()
            .find(|s| s.kind == ScopeKind::TopLevel)
            .unwrap();
        // x should NOT be referenced (def is a hard boundary, can't see x)
        assert_eq!(top.vars["x"].num_references, 0);
    }

    // ── before_declaring_variable hook tests ───────────────────────────

    #[test]
    fn test_block_param_shadows_outer() {
        let (_, decls) = run_with_consumer("x = 1\n[1].each { |x| puts x }\n");
        // The second declaration of 'x' (block param) should see outer_exists = true
        let x_decls: Vec<_> = decls.iter().filter(|(n, _)| n == b"x").collect();
        assert_eq!(x_decls.len(), 2);
        assert!(!x_decls[0].1); // first x = 1, no outer
        assert!(x_decls[1].1); // block param x, outer exists
    }

    #[test]
    fn test_no_shadow_in_def() {
        let (_, decls) = run_with_consumer("x = 1\ndef foo(x)\nend\n");
        let x_decls: Vec<_> = decls.iter().filter(|(n, _)| n == b"x").collect();
        assert_eq!(x_decls.len(), 2);
        assert!(!x_decls[0].1); // first x = 1
        assert!(!x_decls[1].1); // def param x — hard scope, no outer visible
    }

    #[test]
    fn test_nested_block_param_sees_outer_block_param_in_hash_value() {
        let (_, decls) = run_with_consumer(
            r#"def build(typename)
  if typename.to_s.include?('monthly')
    post_process_replace do |res|
      res
    end
  else
    post_process_replace do |res|
      {
        'gcm' => res.map { |res| [res['gcm'], res['annualData'].first] }.to_h
      }
    end
  end
end
"#,
        );
        let res_decls: Vec<_> = decls.iter().filter(|(n, _)| n == b"res").collect();
        assert_eq!(res_decls.len(), 3, "{res_decls:?}");
        assert!(!res_decls[0].1, "{res_decls:?}");
        assert!(!res_decls[1].1, "{res_decls:?}");
        assert!(res_decls[2].1, "{res_decls:?}");
    }

    #[test]
    fn test_nested_block_param_sees_prior_branch_local_inside_call_args() {
        let (_, decls) = run_with_consumer(
            r#"def build(args, target)
  args.map do |modname|
    if modname.type == :hash && modname.children.all? { |pair| pair.children.first.type == :prop }
      ok
    else
      pair = modname.children.first
      s(:send, s(:const, nil, :Object), :defineProperties, target,
        s(:hash, *modname.children.map { |pair|
          s(:pair, s(:sym, pair.children.first.children.last), pair)
        }))
    end
  end
end
"#,
        );
        let pair_decls: Vec<_> = decls.iter().filter(|(n, _)| n == b"pair").collect();
        assert_eq!(pair_decls.len(), 3, "{pair_decls:?}");
        assert!(!pair_decls[0].1, "{pair_decls:?}");
        assert!(!pair_decls[1].1, "{pair_decls:?}");
        assert!(pair_decls[2].1, "{pair_decls:?}");
    }

    // ── Defs (singleton method) tests ──────────────────────────────────

    #[test]
    fn test_defs_receiver_in_outer_scope() {
        let scopes = run_engine("obj = Object.new\ndef obj.foo\n  x = 1\nend\n");
        let top = scopes
            .iter()
            .find(|s| s.kind == ScopeKind::TopLevel)
            .unwrap();
        assert!(top.vars["obj"].num_references > 0);
    }

    #[test]
    fn test_defs_scope_kind() {
        let scopes = run_engine("def self.foo(x)\nend\n");
        let defs = scopes.iter().find(|s| s.kind == ScopeKind::Defs).unwrap();
        assert!(defs.vars.contains_key("x"));
    }

    // ── Branch exclusivity tests ───────────────────────────────────────

    #[test]
    fn test_if_then_else_exclusive_assignments() {
        // x assigned in both branches, neither read → both assignments are useless
        let scopes = run_engine("def foo\n  if cond\n    x = 1\n  else\n    x = 2\n  end\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.num_assignments, 2);
        // Neither assignment is referenced (no read after the if)
        assert!(!x.assignments[0].referenced);
        assert!(!x.assignments[1].referenced);
    }

    #[test]
    fn test_if_then_read_after_if() {
        // x assigned in if-then, read AFTER the if → assignment IS referenced
        let scopes = run_engine("def foo\n  if cond\n    x = 1\n  end\n  puts x\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.num_assignments, 1);
        assert!(x.assignments[0].referenced);
    }

    #[test]
    fn test_if_both_branches_assign_read_after() {
        // x assigned in both branches, read after → both assignments referenced
        let scopes =
            run_engine("def foo\n  if cond\n    x = 1\n  else\n    x = 2\n  end\n  puts x\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.num_assignments, 2);
        // Both should be referenced (read after the if)
        assert!(x.assignments[0].referenced || x.assignments[1].referenced);
    }

    #[test]
    fn test_sibling_branch_read_does_not_reference_exclusive_assignment() {
        let scopes =
            run_engine("def foo\n  x = 0\n  if cond\n    x = 1\n  else\n    x\n  end\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.num_assignments, 2);
        assert!(
            x.assignments[0].referenced,
            "the pre-branch assignment should feed the else-branch read"
        );
        assert!(
            !x.assignments[1].referenced,
            "the then-branch assignment must stay dead across the exclusive else read"
        );
    }

    #[test]
    fn test_predicate_assignment_reaches_guarded_body() {
        let scopes = run_engine("def foo\n  a = nil\n  puts a if (a = 123)\nend\n");
        let def_scope = &scopes[0];
        let a = &def_scope.vars["a"];
        assert_eq!(a.num_assignments, 2);
        assert!(
            a.assignments[0].referenced,
            "modifier-if should keep the older assignment live for scope visibility"
        );
        assert!(
            a.assignments[1].referenced,
            "the predicate assignment should stay live for the guarded body"
        );
    }

    #[test]
    fn test_regular_if_predicate_assignment_overwrites_initializer() {
        let scopes = run_engine("def foo\n  a = nil\n  if a = 123\n    puts a\n  end\nend\n");
        let def_scope = &scopes[0];
        let a = &def_scope.vars["a"];
        assert_eq!(a.num_assignments, 2);
        assert!(
            !a.assignments[0].referenced,
            "a normal if predicate assignment should not keep the initializer live"
        );
        assert!(
            a.assignments[1].referenced,
            "the predicate assignment should still feed the if body"
        );
    }

    #[test]
    fn test_outer_modifier_body_keeps_inner_if_predicate_live() {
        let scopes =
            run_engine("def foo(&block)\n  if (roots = parse)\n    roots\n  end if block\nend\n");
        let def_scope = &scopes[0];
        let roots = &def_scope.vars["roots"];
        assert_eq!(roots.num_assignments, 1);
        assert!(
            roots.assignments[0].referenced,
            "the inner if predicate assignment should still feed reads in the outer modifier body"
        );
    }

    #[test]
    fn test_elsif_predicate_keeps_earlier_siblings_live() {
        let scopes = run_engine(
            "def foo(flag, cached)\n  if flag == :imm\n    reg = 1\n  elsif flag == :indexed\n    reg = 2\n  elsif reg = cached\n    touch(reg)\n  else\n    reg = 3\n  end\n  use(reg)\nend\n",
        );
        let def_scope = &scopes[0];
        let reg = &def_scope.vars["reg"];
        assert_eq!(reg.num_assignments, 4);
        assert!(
            reg.assignments
                .iter()
                .all(|assignment| assignment.referenced),
            "later elsif predicate assignments must not suppress earlier sibling branches"
        );
    }

    #[test]
    fn test_case_branch_survives_nested_predicate_assignment() {
        let scopes = run_engine(
            "def foo(kind, source)\n  case kind\n  when :a\n    r = 1\n  when :b\n    r = 2\n  when :c\n    if (r = source)\n      consume(r)\n    end\n  end\n  puts r\nend\n",
        );
        let def_scope = &scopes[0];
        let r = &def_scope.vars["r"];
        assert_eq!(r.num_assignments, 3);
        assert!(
            r.assignments.iter().all(|assignment| assignment.referenced),
            "a nested predicate assignment in one case branch must not kill sibling branches"
        );
    }

    #[test]
    fn test_if_then_else_different_branch_ids() {
        // Assignments in then vs else should have different branch IDs
        let scopes = run_engine("def foo\n  if cond\n    x = 1\n  else\n    x = 2\n  end\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.num_assignments, 2);
        let bid0 = x.assignments[0].branch_id;
        let bid1 = x.assignments[1].branch_id;
        assert!(
            bid0.is_some(),
            "then-branch assignment should have branch_id"
        );
        assert!(
            bid1.is_some(),
            "else-branch assignment should have branch_id"
        );
        assert_ne!(bid0, bid1, "then and else should have different branch IDs");
    }

    #[test]
    fn test_assignment_outside_branch_has_no_branch_id() {
        let scopes = run_engine("def foo\n  x = 1\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert!(x.assignments[0].branch_id.is_none());
    }

    #[test]
    fn test_reassignment_same_branch_marks_previous_reassigned() {
        // Two assignments in same branch → first is reassigned
        let scopes = run_engine("def foo\n  x = 1\n  x = 2\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.num_assignments, 2);
        assert!(x.assignments[0].reassigned);
        assert!(!x.assignments[1].reassigned);
    }

    #[test]
    fn test_reassignment_different_branches_not_marked_reassigned() {
        // Assignments in exclusive branches → neither is reassigned
        let scopes = run_engine("def foo\n  if cond\n    x = 1\n  else\n    x = 2\n  end\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.num_assignments, 2);
        assert!(
            !x.assignments[0].reassigned,
            "then-branch assignment should NOT be marked reassigned"
        );
        assert!(
            !x.assignments[1].reassigned,
            "else-branch assignment should NOT be marked reassigned"
        );
    }

    // ── Loop back-edge tests ───────────────────────────────────────────

    #[test]
    fn test_while_loop_back_edge() {
        // x assigned and read in while loop → assignment marked referenced (back-edge)
        let scopes = run_engine("def foo\n  while cond\n    x = compute\n    puts x\n  end\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert!(
            x.assignments[0].referenced,
            "loop assignment should be referenced via back-edge or direct read"
        );
    }

    #[test]
    fn test_for_loop_variable_referenced() {
        // for loop index is referenced in body
        let scopes = run_engine("def foo\n  for i in [1,2,3]\n    puts i\n  end\nend\n");
        let def_scope = &scopes[0];
        let i = &def_scope.vars["i"];
        assert!(i.used);
    }

    // ── Case/when branch tests ─────────────────────────────────────────

    #[test]
    fn test_case_when_branches_are_exclusive() {
        let scopes = run_engine(
            "def foo(v)\n  case v\n  when 1\n    x = :a\n  when 2\n    x = :b\n  end\nend\n",
        );
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.num_assignments, 2);
        // Neither should be marked reassigned (exclusive branches)
        assert!(!x.assignments[0].reassigned);
        assert!(!x.assignments[1].reassigned);
    }

    // ── Useless assignment pattern tests ───────────────────────────────

    #[test]
    fn test_useless_assignment_detected() {
        // x = 1 is useless because x = 2 overwrites it before any read
        let scopes = run_engine("def foo\n  x = 1\n  x = 2\n  puts x\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.num_assignments, 2);
        // First assignment: reassigned and NOT referenced → useless
        assert!(x.assignments[0].reassigned);
        assert!(!x.assignments[0].referenced);
        // Second assignment: referenced → useful
        assert!(x.assignments[1].referenced);
    }

    #[test]
    fn test_assignment_used_then_overwritten() {
        // x = 1 is used (puts x), then x = 2 is useless (never read)
        let scopes = run_engine("def foo\n  x = 1\n  puts x\n  x = 2\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.num_assignments, 2);
        assert!(
            x.assignments[0].referenced,
            "first assignment should be referenced"
        );
        assert!(
            !x.assignments[1].referenced,
            "second assignment should NOT be referenced (useless)"
        );
    }

    #[test]
    fn test_begin_rescue_assignment_not_useless() {
        // result = nil; begin; result = compute; rescue; end; puts result
        // The first assignment is NOT useless because rescue may execute before
        // the second assignment completes.
        let scopes = run_engine(
            "def foo\n  result = nil\n  begin\n    result = compute\n  rescue\n    puts result\n  end\nend\n",
        );
        let def_scope = &scopes[0];
        let result = &def_scope.vars["result"];
        // The nil assignment should be referenced (rescue reads it)
        // or at least not marked as reassigned (conservative)
        let first = &result.assignments[0];
        assert!(
            first.referenced || !first.reassigned,
            "result = nil should not be flagged as useless (rescue may use it)"
        );
    }

    // ── Rescue exception capture tests ────────────────────────────────

    #[test]
    fn test_rescue_exception_capture_tracked() {
        // `rescue StandardError => e; puts e.message` — the exception variable
        // should be declared, assigned with ExceptionCapture, and referenced.
        let scopes = run_engine(
            "def foo\n  begin\n    risky\n  rescue StandardError => e\n    puts e.message\n  end\nend\n",
        );
        let def_scope = &scopes[0];
        assert!(
            def_scope.vars.contains_key("e"),
            "rescue exception variable 'e' should be tracked"
        );
        let e = &def_scope.vars["e"];
        assert_eq!(e.num_assignments, 1, "should have exactly 1 assignment");
        assert_eq!(
            e.assignments[0].kind,
            crate::cop::variable_force::assignment::AssignmentKind::ExceptionCapture,
            "assignment should be ExceptionCapture"
        );
        assert!(
            e.num_references > 0,
            "e should be referenced by puts e.message"
        );
    }

    #[test]
    fn test_rescue_exception_capture_unused() {
        // `rescue StandardError => e; puts "error"` — exception variable assigned
        // but never read.
        let scopes = run_engine(
            "def foo\n  begin\n    risky\n  rescue StandardError => e\n    puts \"error\"\n  end\nend\n",
        );
        let def_scope = &scopes[0];
        let e = &def_scope.vars["e"];
        assert_eq!(e.num_assignments, 1);
        assert_eq!(e.num_references, 0, "e should have no references");
    }

    #[test]
    fn test_rescue_chained_clauses() {
        // Multiple rescue clauses each with their own exception variable.
        let scopes = run_engine(
            "def foo\n  begin\n    risky\n  rescue TypeError => e1\n    puts e1\n  rescue => e2\n    puts e2\n  end\nend\n",
        );
        let def_scope = &scopes[0];
        assert!(def_scope.vars.contains_key("e1"), "e1 should be tracked");
        assert!(def_scope.vars.contains_key("e2"), "e2 should be tracked");
        assert!(def_scope.vars["e1"].num_references > 0);
        assert!(def_scope.vars["e2"].num_references > 0);
    }

    #[test]
    fn test_multi_rescue_self_reference_keeps_clauses_exclusive() {
        let scopes = run_engine(
            "def foo(sock_obj)\n  begin\n    work\n  rescue Errno::ECONNRESET\n    sock_obj = disconnect(sock_obj) unless sock_obj.nil?\n  rescue StandardError\n    sock_obj = disconnect(sock_obj) unless sock_obj.nil?\n  end\nend\n",
        );
        let def_scope = &scopes[0];
        let sock_obj = &def_scope.vars["sock_obj"];

        assert_eq!(sock_obj.num_assignments, 2);
        assert_ne!(
            sock_obj.assignments[0].branch_id, sock_obj.assignments[1].branch_id,
            "each rescue clause should get its own sibling branch"
        );
        assert!(
            !sock_obj.assignments[0].referenced,
            "the later rescue clause must not keep the earlier clause alive"
        );
        assert!(
            !sock_obj.assignments[1].referenced,
            "the final rescue-clause write is also useless here"
        );
    }

    #[test]
    fn test_nested_rescue_reraise_keeps_inner_capture_live() {
        let scopes = run_engine(
            "def foo\n  begin\n    begin\n      raise \"Error 1\"\n    rescue => e1\n      raise \"Error 2\"\n    end\n  rescue => e2\n    e2.cause == e1\n    raise e2\n  rescue => e\n    e.cause == e1\n  end\nend\n",
        );
        let def_scope = &scopes[0];

        assert!(
            def_scope.vars["e1"].assignments[0].referenced,
            "outer rescue reads should keep the inner rescue capture live"
        );
        assert!(
            def_scope.vars["e2"].assignments[0].referenced,
            "re-raising the outer rescue capture should keep it live"
        );
    }

    // ── Pattern match variable tests ──────────────────────────────────

    #[test]
    fn test_pattern_match_variable_tracked() {
        // `case v; in x; puts x; end` — pattern match variable should be
        // declared with PatternMatch kind, assigned, and referenced.
        let scopes = run_engine("def foo(v)\n  case v\n  in x\n    puts x\n  end\nend\n");
        let def_scope = &scopes[0];
        assert!(
            def_scope.vars.contains_key("x"),
            "pattern match variable 'x' should be tracked"
        );
        let x = &def_scope.vars["x"];
        assert_eq!(x.decl_kind, DeclarationKind::PatternMatch);
        assert_eq!(x.num_assignments, 1, "should have exactly 1 assignment");
        assert!(x.num_references > 0, "x should be referenced by puts x");
    }

    #[test]
    fn test_pattern_match_variable_unused() {
        // `case v; in x; "matched"; end` — pattern variable assigned but not read.
        let scopes = run_engine("def foo(v)\n  case v\n  in x\n    \"matched\"\n  end\nend\n");
        let def_scope = &scopes[0];
        let x = &def_scope.vars["x"];
        assert_eq!(x.decl_kind, DeclarationKind::PatternMatch);
        assert_eq!(x.num_references, 0, "x should have no references");
    }

    #[test]
    fn test_pattern_match_guard_clause() {
        // `case v; in _ if _.blank?; 42; end` — guard references the variable
        // before the pattern target declares it. Pre-declaration should ensure
        // the reference is tracked.
        let scopes = run_engine("def foo(v)\n  case v\n  in _ if _.blank?\n    42\n  end\nend\n");
        let def_scope = &scopes[0];
        assert!(
            def_scope.vars.contains_key("_"),
            "pattern match variable '_' should be tracked"
        );
        let underscore = &def_scope.vars["_"];
        assert!(
            underscore.num_references > 0,
            "_ should be referenced by guard clause _.blank?"
        );
    }

    // ── Nested multi-write should NOT create extra assignments ────────

    #[test]
    fn test_nested_multi_write_no_extra_assignments() {
        // `a, (b, c) = 1, [2, 3]` — nested multi-write targets b and c should
        // NOT be tracked by the VF engine (matching RuboCop behavior). Only
        // top-level target 'a' should have an assignment.
        let scopes = run_engine("a, (b, c) = 1, [2, 3]\nputs a\n");
        let top = &scopes[0];
        assert!(
            top.vars.contains_key("a"),
            "top-level target 'a' should be tracked"
        );
        assert_eq!(top.vars["a"].num_assignments, 1);
        // Nested targets should NOT be tracked (no generic handler to catch them)
        assert!(
            !top.vars.contains_key("b"),
            "nested multi-write target 'b' should not be tracked"
        );
        assert!(
            !top.vars.contains_key("c"),
            "nested multi-write target 'c' should not be tracked"
        );
    }
}
