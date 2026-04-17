use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use ruby_prism::Visit;

use crate::cop::variable_force::engine::{Engine, RegisteredConsumer};
use crate::cop::variable_force::{self, Scope, VariableTable};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Checks for every useless assignment to local variable in every scope.
///
/// ## Implementation
///
/// This cop runs VariableForce inside `check_source`, then post-processes the
/// pending offenses before emitting diagnostics. The shared engine still does
/// the variable lifetime analysis; this wrapper only filters known
/// RuboCop-compatible false positives.
///
/// ## Rescue-clause branch parity (2026-04-17)
///
/// Earlier VariableForce builds visited an entire `begin ... rescue ...
/// rescue ... end` chain under one branch context. That let a later rescue
/// clause's self-reference or write keep an earlier rescue-clause assignment
/// alive, missing offenses like repeated
/// `sock_obj = disconnect(sock_obj) unless sock_obj.nil?`.
///
/// The engine now models exception handlers like RuboCop: the `begin` body is
/// an incomplete branch that may jump into sibling rescue/else paths, while
/// each rescue clause and the `else` body get their own sibling branch. This
/// wrapper still keeps a narrow multi-rescue fallback for parity when a later
/// read outside the rescue chain legitimately uses one of those clause values.
///
/// ## Nested block-scope branch boundaries (2026-04-17)
///
/// Branch ancestry must stop at the variable's own scope. Without that, outer
/// `begin/rescue` branch metadata leaked into proc-local variables and kept
/// dead initializers like `test_request = nil` alive inside nested `proc`
/// bodies. VariableForce now trims branch paths to the declaring scope before
/// comparing assignments and references.
///
/// ## FP fix: pattern matching captures (2026-04-04)
///
/// RuboCop does not flag variables captured in pattern matching
/// (`case/in` or rightward `expr => pattern`, e.g. `in [_, middle, *rest]`
/// or `[1, 2] => [first, *rest]`). The variable_force engine creates
/// assignments for these captures, but they should never be reported as
/// useless. Fixed by collecting all pattern match target offsets from the
/// AST and skipping those offsets during offense emission.
///
/// ## FP fix: rescue body assignments read in handlers (2026-04-11)
///
/// Assignments in begin/rescue/ensure bodies are not useless when the variable
/// is read in the rescue or ensure handler — if a later overwrite raises, the
/// handler reads the earlier value. The previous suppression required
/// `captured_by_block` to be true, which missed the common case of a plain
/// variable (no block capture) read in a rescue/ensure handler (e.g.,
/// `code = X; code = Y; rescue; Result.new(code:); end`). Fixed by making the
/// suppression name-aware: collect variable names read in rescue/ensure handlers
/// and suppress body writes for those names, regardless of block capture.
///
/// ## FN fix: live branch contexts during traversal (2026-04-16)
///
/// VariableForce was only copying branch-context metadata into
/// `VariableTable` at scope exit, after all references had already been
/// resolved. During the actual walk, sibling-branch reads therefore looked
/// compatible and incorrectly kept exclusive assignments alive, missing cases
/// like `if cond; guid = foo; else; guid; end`. Fixed by keeping the
/// `VariableTable` branch contexts in sync as branches are created, while
/// treating predicate-assignment contexts as visible to their guarded bodies
/// so patterns like `puts a if (a = 123)` still match RuboCop.
pub struct UselessAssignment;

impl Cop for UselessAssignment {
    fn name(&self) -> &'static str {
        "Lint/UselessAssignment"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
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
        let collector = PendingOffenseCollector::default();
        let consumers = [RegisteredConsumer {
            consumer: &collector,
            config,
        }];
        let mut engine = Engine::new(source, &consumers);
        engine.run(parse_result);
        let _ = engine.into_diagnostics();

        let rescue_contexts = collect_multi_rescue_contexts(parse_result);
        let conditional_operator_offsets = collect_conditional_operator_write_offsets(parse_result);
        let pattern_match_offsets = collect_pattern_match_target_offsets(parse_result);
        let or_condition_offsets = collect_or_condition_write_offsets(parse_result);
        let do_while_body_offsets = collect_do_while_body_write_offsets(parse_result);
        let rescue_body_write_offsets = collect_rescue_body_write_offsets(parse_result);
        let mut rescue_modifier_collector = RescueModifierWriteCollector::default();
        rescue_modifier_collector.visit(&parse_result.node());
        let mut rescue_modifier_offsets = rescue_modifier_collector.offsets;
        // Also suppress preceding writes to variables that are written in rescue modifiers
        let preceding = collect_rescue_modifier_preceding_write_offsets(
            parse_result,
            &rescue_modifier_collector.rescue_value_var_names,
        );
        rescue_modifier_offsets.extend(preceding);
        let mut candidates = collector.take_candidates();
        candidates.sort_by_key(|candidate| candidate.node_offset);

        for candidate in candidates {
            if pattern_match_offsets.contains(&candidate.node_offset) {
                continue;
            }

            let emit = if conditional_operator_offsets.contains(&candidate.node_offset) {
                true
            } else if !candidate.engine_used {
                // Suppress if inside rescue body and variable is read in
                // rescue/ensure handler: RuboCop's VF branch model makes
                // rescue-body references reach earlier assignments, keeping
                // them alive. Our VF engine doesn't model rescue branches,
                // so we suppress these in post-processing.
                if rescue_body_write_offsets.contains(&candidate.node_offset) {
                    false
                } else {
                    !should_suppress_multi_rescue_false_positive(&candidate, &rescue_contexts)
                        && !or_condition_offsets.contains(&candidate.node_offset)
                        && !do_while_body_offsets.contains(&candidate.node_offset)
                        && !rescue_modifier_offsets.contains(&candidate.node_offset)
                }
            } else {
                false
            };

            if !emit {
                continue;
            }

            let (line, column) = source.offset_to_line_col(candidate.node_offset);
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                format!(
                    "Useless assignment to variable - `{}`.",
                    String::from_utf8_lossy(&candidate.name)
                ),
            ));
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AssignmentState {
    offset: usize,
    branch_id: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct ReferenceState {
    offset: usize,
    branch_id: Option<usize>,
}

#[derive(Debug, Clone)]
struct AssignmentCandidate {
    name: Vec<u8>,
    node_offset: usize,
    branch_id: Option<usize>,
    engine_used: bool,
    value_range: Option<(usize, usize)>,
    assignment_states: Vec<AssignmentState>,
    reference_states: Vec<ReferenceState>,
}

#[derive(Default)]
struct PendingOffenseCollector {
    candidates: Mutex<Vec<AssignmentCandidate>>,
}

impl PendingOffenseCollector {
    fn take_candidates(&self) -> Vec<AssignmentCandidate> {
        std::mem::take(&mut *self.candidates.lock().unwrap())
    }
}

impl variable_force::VariableForceConsumer for PendingOffenseCollector {
    fn before_leaving_scope(
        &self,
        scope: &Scope,
        _variable_table: &VariableTable,
        _source: &SourceFile,
        _config: &CopConfig,
        _diagnostics: &mut Vec<Diagnostic>,
    ) {
        let mut candidates = self.candidates.lock().unwrap();

        for variable in scope.variables.values() {
            if variable.should_be_unused() {
                continue;
            }

            let assignment_states: Vec<_> = variable
                .assignments
                .iter()
                .map(|assignment| AssignmentState {
                    offset: assignment.node_offset,
                    branch_id: assignment.branch_id,
                })
                .collect();
            let reference_states: Vec<_> = variable
                .references
                .iter()
                .map(|reference| ReferenceState {
                    offset: reference.node_offset,
                    branch_id: reference.branch_id,
                })
                .collect();
            for assignment in &variable.assignments {
                candidates.push(AssignmentCandidate {
                    name: variable.name.clone(),
                    node_offset: assignment.node_offset,
                    branch_id: assignment.branch_id,
                    engine_used: assignment.used(variable.captured_by_block),
                    value_range: assignment.value_range,
                    assignment_states: assignment_states.clone(),
                    reference_states: reference_states.clone(),
                });
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RescueClauseContext {
    begin_offset: usize,
    clause_index: usize,
}

#[derive(Default)]
struct MultiRescueContexts {
    assignments: HashMap<usize, RescueClauseContext>,
    references: HashMap<usize, RescueClauseContext>,
}

fn collect_multi_rescue_contexts(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> MultiRescueContexts {
    let mut collector = MultiRescueCollector::default();
    collector.visit(&parse_result.node());
    collector.contexts
}

#[derive(Default)]
struct MultiRescueCollector {
    contexts: MultiRescueContexts,
    rescue_stack: Vec<RescueClauseContext>,
}

impl MultiRescueCollector {
    fn visit_rescue_clause_body(
        &mut self,
        rescue: ruby_prism::RescueNode<'_>,
        begin_offset: usize,
        clause_index: usize,
        multi_clause: bool,
    ) {
        if multi_clause {
            self.rescue_stack.push(RescueClauseContext {
                begin_offset,
                clause_index,
            });
        }

        for exception in rescue.exceptions().iter() {
            self.visit(&exception);
        }
        if let Some(reference) = rescue.reference() {
            self.visit(&reference);
        }
        if let Some(statements) = rescue.statements() {
            for statement in statements.body().iter() {
                self.visit(&statement);
            }
        }

        if multi_clause {
            self.rescue_stack.pop();
        }
    }
}

impl<'pr> Visit<'pr> for MultiRescueCollector {
    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        if let Some(statements) = node.statements() {
            for statement in statements.body().iter() {
                self.visit(&statement);
            }
        }

        if let Some(first_rescue) = node.rescue_clause() {
            let begin_offset = node.location().start_offset();
            let mut clauses = Vec::new();
            let mut current = Some(first_rescue);
            while let Some(rescue) = current {
                let next = rescue.subsequent();
                clauses.push(rescue);
                current = next;
            }

            let multi_clause = clauses.len() > 1;
            for (clause_index, rescue) in clauses.into_iter().enumerate() {
                self.visit_rescue_clause_body(rescue, begin_offset, clause_index, multi_clause);
            }
        }

        if let Some(else_clause) = node.else_clause() {
            if let Some(statements) = else_clause.statements() {
                for statement in statements.body().iter() {
                    self.visit(&statement);
                }
            }
        }

        if let Some(ensure_clause) = node.ensure_clause() {
            if let Some(statements) = ensure_clause.statements() {
                for statement in statements.body().iter() {
                    self.visit(&statement);
                }
            }
        }
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        if let Some(context) = self.rescue_stack.last().copied() {
            self.contexts
                .assignments
                .insert(node.location().start_offset(), context);
        }
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        if let Some(context) = self.rescue_stack.last().copied() {
            self.contexts
                .references
                .insert(node.location().start_offset(), context);
        }
    }
}

fn should_suppress_multi_rescue_false_positive(
    offense: &AssignmentCandidate,
    contexts: &MultiRescueContexts,
) -> bool {
    let Some(context) = contexts.assignments.get(&offense.node_offset).copied() else {
        return false;
    };

    let has_later_sibling_assignment = offense.assignment_states.iter().any(|assignment| {
        assignment.offset > offense.node_offset
            && is_sibling_multi_rescue_assignment(context, assignment.offset, contexts)
    });
    if !has_later_sibling_assignment {
        return false;
    }

    let next_real_assignment = offense
        .assignment_states
        .iter()
        .copied()
        .filter(|assignment| assignment.offset > offense.node_offset)
        .filter(|assignment| {
            !is_sibling_multi_rescue_assignment(context, assignment.offset, contexts)
                && (assignment.branch_id.is_none() || assignment.branch_id == offense.branch_id)
        })
        .map(|assignment| assignment.offset)
        .min()
        .unwrap_or(usize::MAX);

    offense
        .reference_states
        .iter()
        .filter(|reference| {
            reference.offset > offense.node_offset && reference.offset < next_real_assignment
        })
        .filter(|reference| !offset_in_value_range(reference.offset, offense.value_range))
        .any(|reference| {
            !is_sibling_multi_rescue_reference(context, reference.offset, contexts)
                && reference_can_consume_rescue_value(*reference, offense.branch_id)
        })
}

fn offset_in_value_range(offset: usize, range: Option<(usize, usize)>) -> bool {
    range.is_some_and(|(start, end)| start <= offset && offset < end)
}

fn is_sibling_multi_rescue_assignment(
    current: RescueClauseContext,
    offset: usize,
    contexts: &MultiRescueContexts,
) -> bool {
    contexts.assignments.get(&offset).is_some_and(|other| {
        other.begin_offset == current.begin_offset && other.clause_index != current.clause_index
    })
}

fn is_sibling_multi_rescue_reference(
    current: RescueClauseContext,
    offset: usize,
    contexts: &MultiRescueContexts,
) -> bool {
    contexts.references.get(&offset).is_some_and(|other| {
        other.begin_offset == current.begin_offset && other.clause_index != current.clause_index
    })
}

fn reference_can_consume_rescue_value(
    reference: ReferenceState,
    offense_branch_id: Option<usize>,
) -> bool {
    reference.branch_id.is_none() || reference.branch_id == offense_branch_id
}

fn collect_conditional_operator_write_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> HashSet<usize> {
    let mut collector = ConditionalOperatorWriteCollector::default();
    collector.visit(&parse_result.node());
    collector.offsets
}

#[derive(Default)]
struct ConditionalOperatorWriteCollector {
    offsets: HashSet<usize>,
}

impl<'pr> Visit<'pr> for ConditionalOperatorWriteCollector {
    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        if let (Some(if_body), Some(subsequent)) = (node.statements(), node.subsequent()) {
            if let Some(else_node) = subsequent.as_else_node() {
                let if_stmt = single_statement_from_statements(&if_body);
                let else_stmt = else_node
                    .statements()
                    .and_then(|statements| single_statement_from_statements(&statements));

                if let (Some(if_stmt), Some(else_stmt)) = (if_stmt, else_stmt) {
                    if let Some(offset) = matching_operator_write_offset(&if_stmt, &else_stmt) {
                        self.offsets.insert(offset);
                    }
                    if let Some(offset) = matching_operator_write_offset(&else_stmt, &if_stmt) {
                        self.offsets.insert(offset);
                    }
                }
            }
        }

        ruby_prism::visit_if_node(self, node);
    }
}

fn single_statement_from_statements<'pr>(
    statements: &ruby_prism::StatementsNode<'pr>,
) -> Option<ruby_prism::Node<'pr>> {
    let mut body = statements.body().iter();
    let statement = body.next()?;
    if body.next().is_some() {
        return None;
    }
    Some(statement)
}

fn matching_operator_write_offset(
    write_branch: &ruby_prism::Node<'_>,
    read_branch: &ruby_prism::Node<'_>,
) -> Option<usize> {
    let write = write_branch.as_local_variable_operator_write_node()?;
    let read = read_branch.as_local_variable_read_node()?;
    if write.name().as_slice() == read.name().as_slice() {
        Some(write.location().start_offset())
    } else {
        None
    }
}

fn collect_pattern_match_target_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> HashSet<usize> {
    let mut collector = PatternMatchTargetCollector::default();
    collector.visit(&parse_result.node());
    collector.offsets
}

#[derive(Default)]
struct PatternMatchTargetCollector {
    offsets: HashSet<usize>,
    in_pattern: bool,
}

impl<'pr> Visit<'pr> for PatternMatchTargetCollector {
    fn visit_in_node(&mut self, node: &ruby_prism::InNode<'pr>) {
        let was_in_pattern = self.in_pattern;
        self.in_pattern = true;
        self.visit(&node.pattern());
        self.in_pattern = false;
        // Visit the body and guard normally (not in pattern context)
        if let Some(stmts) = node.statements() {
            for stmt in stmts.body().iter() {
                self.visit(&stmt);
            }
        }
        self.in_pattern = was_in_pattern;
    }

    fn visit_match_required_node(&mut self, node: &ruby_prism::MatchRequiredNode<'pr>) {
        self.visit(&node.value());
        let was_in_pattern = self.in_pattern;
        self.in_pattern = true;
        self.visit(&node.pattern());
        self.in_pattern = was_in_pattern;
    }

    fn visit_local_variable_target_node(
        &mut self,
        node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        if self.in_pattern {
            self.offsets.insert(node.location().start_offset());
        }
    }
}

// ---------------------------------------------------------------------------
// FP suppression: assignments inside || / or expressions
// ---------------------------------------------------------------------------
//
// In `if foo(x = 1) || foo(x = 2)`, short-circuit evaluation means only one
// branch executes, but the VF engine sees both assignments as sequential in
// the same scope. Suppress the LHS assignment when the same variable is also
// assigned in the RHS of an `or`/`||` node.

fn collect_or_condition_write_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> HashSet<usize> {
    let mut collector = OrConditionWriteCollector::default();
    collector.visit(&parse_result.node());
    collector.offsets
}

#[derive(Default)]
struct OrConditionWriteCollector {
    offsets: HashSet<usize>,
}

/// Helper visitor that collects all local variable write offsets within a subtree.
#[derive(Default)]
struct LvarWriteSubtreeCollector {
    writes: Vec<(Vec<u8>, usize)>,
}

impl<'pr> Visit<'pr> for LvarWriteSubtreeCollector {
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        self.writes.push((
            node.name().as_slice().to_vec(),
            node.location().start_offset(),
        ));
        ruby_prism::visit_local_variable_write_node(self, node);
    }
}

impl OrConditionWriteCollector {
    fn process_or_node(&mut self, node: &ruby_prism::OrNode<'_>) {
        let mut lhs_collector = LvarWriteSubtreeCollector::default();
        lhs_collector.visit(&node.left());
        let mut rhs_collector = LvarWriteSubtreeCollector::default();
        rhs_collector.visit(&node.right());

        // For each variable written in LHS that is also written in RHS,
        // suppress the LHS write offset.
        for (lhs_name, lhs_offset) in &lhs_collector.writes {
            if rhs_collector
                .writes
                .iter()
                .any(|(rhs_name, _)| rhs_name == lhs_name)
            {
                self.offsets.insert(*lhs_offset);
            }
        }
    }
}

impl<'pr> Visit<'pr> for OrConditionWriteCollector {
    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        self.process_or_node(node);
        ruby_prism::visit_or_node(self, node);
    }
}

// ---------------------------------------------------------------------------
// FP suppression: assignments inside begin/end until or begin/end while loops
// ---------------------------------------------------------------------------
//
// `begin ... end until cond` is a do-while loop: the body executes at least
// once before the condition is checked. The VF engine may flag a write inside
// the body as unused if the only read is in the loop condition, because it
// processes the condition before the body.

fn collect_do_while_body_write_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> HashSet<usize> {
    let mut collector = DoWhileBodyWriteCollector::default();
    collector.visit(&parse_result.node());
    collector.offsets
}

#[derive(Default)]
struct DoWhileBodyWriteCollector {
    offsets: HashSet<usize>,
}

/// Helper visitor that collects all local variable read names within a subtree.
#[derive(Default)]
struct LvarReadSubtreeCollector {
    names: HashSet<Vec<u8>>,
}

impl<'pr> Visit<'pr> for LvarReadSubtreeCollector {
    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        self.names.insert(node.name().as_slice().to_vec());
    }
}

impl DoWhileBodyWriteCollector {
    fn process_loop(
        &mut self,
        predicate: &ruby_prism::Node<'_>,
        body: Option<ruby_prism::StatementsNode<'_>>,
        is_begin_modifier: bool,
    ) {
        if !is_begin_modifier {
            return;
        }
        let mut cond_reader = LvarReadSubtreeCollector::default();
        cond_reader.visit(predicate);
        if cond_reader.names.is_empty() {
            return;
        }
        if let Some(stmts) = body {
            let mut body_writer = LvarWriteSubtreeCollector::default();
            for stmt in stmts.body().iter() {
                body_writer.visit(&stmt);
            }
            for (name, offset) in body_writer.writes {
                if cond_reader.names.contains(&name) {
                    self.offsets.insert(offset);
                }
            }
        }
    }
}

impl<'pr> Visit<'pr> for DoWhileBodyWriteCollector {
    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        self.process_loop(
            &node.predicate(),
            node.statements(),
            node.is_begin_modifier(),
        );
        ruby_prism::visit_until_node(self, node);
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        self.process_loop(
            &node.predicate(),
            node.statements(),
            node.is_begin_modifier(),
        );
        ruby_prism::visit_while_node(self, node);
    }
}

// ---------------------------------------------------------------------------
// FP suppression: assignments inside begin/rescue bodies
// ---------------------------------------------------------------------------
//
// RuboCop's VF branch model creates branches for begin/rescue, making the
// reference walk reach earlier assignments in the rescue-able body. Our VF
// engine treats the body as unbranched, so assignments overwritten before the
// rescue handler look dead even though the handler could read the earlier value.
//
// We suppress rescue-body writes when the same variable is read in the
// rescue/ensure handlers of the same begin block. This matches RuboCop's
// implicit reachability across the begin/rescue branch boundary.

struct RescueBodyInfo {
    /// Variable names (with offsets) of writes in the begin body.
    write_names: Vec<(Vec<u8>, usize)>,
    /// Variable names read in rescue/ensure handlers.
    handler_read_names: HashSet<Vec<u8>>,
}

fn collect_rescue_body_write_offsets(parse_result: &ruby_prism::ParseResult<'_>) -> HashSet<usize> {
    let mut collector = RescueBodyWriteCollector::default();
    collector.visit(&parse_result.node());

    // Build the final offset set: only include writes for variables that
    // are actually read in the rescue/ensure handlers of the same begin block.
    let mut offsets = HashSet::new();
    for info in &collector.begin_blocks {
        for (name, offset) in &info.write_names {
            if info.handler_read_names.contains(name) {
                offsets.insert(*offset);
            }
        }
    }
    offsets
}

#[derive(Default)]
struct RescueBodyWriteCollector {
    begin_blocks: Vec<RescueBodyInfo>,
    in_rescue_body: bool,
    in_handler: bool,
    current_begin_idx: Option<usize>,
}

impl<'pr> Visit<'pr> for RescueBodyWriteCollector {
    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        if node.rescue_clause().is_some() || node.ensure_clause().is_some() {
            let idx = self.begin_blocks.len();
            self.begin_blocks.push(RescueBodyInfo {
                write_names: Vec::new(),
                handler_read_names: HashSet::new(),
            });

            // Visit body as rescue-able
            let prev_body = self.in_rescue_body;
            let prev_idx = self.current_begin_idx;
            self.in_rescue_body = true;
            self.current_begin_idx = Some(idx);
            if let Some(stmts) = node.statements() {
                for stmt in stmts.body().iter() {
                    self.visit(&stmt);
                }
            }
            self.in_rescue_body = prev_body;

            // Visit rescue/else/ensure as handler context
            let prev_handler = self.in_handler;
            self.in_handler = true;
            let mut current_rescue = node.rescue_clause();
            while let Some(rescue) = current_rescue {
                for exception in rescue.exceptions().iter() {
                    self.visit(&exception);
                }
                if let Some(reference) = rescue.reference() {
                    self.visit(&reference);
                }
                if let Some(stmts) = rescue.statements() {
                    for stmt in stmts.body().iter() {
                        self.visit(&stmt);
                    }
                }
                current_rescue = rescue.subsequent();
            }
            if let Some(else_clause) = node.else_clause() {
                if let Some(stmts) = else_clause.statements() {
                    for stmt in stmts.body().iter() {
                        self.visit(&stmt);
                    }
                }
            }
            if let Some(ensure) = node.ensure_clause() {
                if let Some(stmts) = ensure.statements() {
                    for stmt in stmts.body().iter() {
                        self.visit(&stmt);
                    }
                }
            }
            self.in_handler = prev_handler;
            self.current_begin_idx = prev_idx;
        } else {
            ruby_prism::visit_begin_node(self, node);
        }
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        if self.in_rescue_body {
            if let Some(idx) = self.current_begin_idx {
                let offset = node.location().start_offset();
                let name = node.name().as_slice().to_vec();
                self.begin_blocks[idx].write_names.push((name, offset));
            }
        }
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_local_variable_target_node(
        &mut self,
        node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        if self.in_rescue_body {
            if let Some(idx) = self.current_begin_idx {
                let offset = node.location().start_offset();
                let name = node.name().as_slice().to_vec();
                self.begin_blocks[idx].write_names.push((name, offset));
            }
        }
    }

    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        if self.in_handler {
            if let Some(idx) = self.current_begin_idx {
                self.begin_blocks[idx]
                    .handler_read_names
                    .insert(node.name().as_slice().to_vec());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FP suppression: assignments inside rescue modifier expressions
// ---------------------------------------------------------------------------
//
// `x = Float(y) rescue err = "bad"` — the rescue modifier creates an implicit
// branch but the VF engine sees `err = "bad"` as sequential with prior writes.
// Suppress writes inside the rescue value of a RescueModifierNode.

#[derive(Default)]
struct RescueModifierWriteCollector {
    offsets: HashSet<usize>,
    /// Variable names that have writes inside rescue modifier expressions.
    rescue_value_var_names: HashSet<Vec<u8>>,
    in_rescue_value: bool,
}

impl<'pr> Visit<'pr> for RescueModifierWriteCollector {
    fn visit_rescue_modifier_node(&mut self, node: &ruby_prism::RescueModifierNode<'pr>) {
        // Visit the expression (normal path) normally
        self.visit(&node.expression());
        // Visit the rescue value (fallback path) with suppression active
        let was = self.in_rescue_value;
        self.in_rescue_value = true;
        self.visit(&node.rescue_expression());
        self.in_rescue_value = was;
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        if self.in_rescue_value {
            self.offsets.insert(node.location().start_offset());
            self.rescue_value_var_names
                .insert(node.name().as_slice().to_vec());
        }
        ruby_prism::visit_local_variable_write_node(self, node);
    }
}

/// Second pass: suppress writes to variables that are also written inside rescue
/// modifiers. The initial `err = nil` before `x = Float(y) rescue err = "bad"`
/// is a fallback value, not a dead write.
fn collect_rescue_modifier_preceding_write_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
    rescue_value_var_names: &HashSet<Vec<u8>>,
) -> HashSet<usize> {
    if rescue_value_var_names.is_empty() {
        return HashSet::new();
    }
    let mut collector = RescueModifierPrecedingWriteCollector {
        offsets: HashSet::new(),
        names: rescue_value_var_names,
    };
    collector.visit(&parse_result.node());
    collector.offsets
}

struct RescueModifierPrecedingWriteCollector<'a> {
    offsets: HashSet<usize>,
    names: &'a HashSet<Vec<u8>>,
}

impl<'pr> Visit<'pr> for RescueModifierPrecedingWriteCollector<'_> {
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        if self.names.contains(node.name().as_slice()) {
            self.offsets.insert(node.location().start_offset());
        }
        ruby_prism::visit_local_variable_write_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(UselessAssignment, "cops/lint/useless_assignment");
}
