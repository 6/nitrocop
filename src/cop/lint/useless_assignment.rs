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
/// ## FN fix: stale rescue-body suppression removed (2026-04-19)
///
/// Before the rescue/ensure branch model landed in VariableForce, this wrapper
/// kept begin-body writes alive whenever a rescue or ensure handler mentioned
/// the same variable name. After the engine started modeling those branches
/// directly, the wrapper became too broad and hid real offenses where the
/// handler immediately redefined the variable before any read, e.g.
/// `line = __LINE__; raise; rescue => err; file, line = err.source; ...`.
/// The post-filter is gone now; rescue/ensure liveness comes from the engine's
/// branch metadata instead of a name-only fallback.
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
///
/// ## FN fix: normal predicate assignments overwrite prior initializers (2026-04-19)
///
/// RuboCop treats modifier-form conditionals as a scope quirk: `puts a if (a = 123)`
/// keeps the older `a = nil` live so `a` exists on the left of the `if`.
/// nitrocop had modeled every predicate assignment as a branch, which kept dead
/// initializers like `origin = nil` alive before `if origin = input` and then
/// broke sibling-branch liveness in larger `if`/`elsif` and `case` chains.
/// Fixed in VariableForce by matching RuboCop's branch model: normal predicate
/// assignments are unbranched, while modifier-form predicates keep a dedicated
/// context so earlier assignments stay visible on the left side of the keyword.
///
/// ## FN fix: sequential loop writes before first read (2026-04-20)
///
/// RuboCop only gives loop back-edge credit to the last assignment in a plain
/// loop body, plus assignments nested under real branch nodes like `if`,
/// `case`, and `rescue`. nitrocop had treated the loop body itself as a branch,
/// which kept overwritten writes like `pulls = []; pulls = fetch; break if
/// pulls.count == 0` alive and missed real offenses. VariableForce now tracks
/// which branch contexts participate in RuboCop's loop back-edge logic so
/// sequential loop-body overwrites remain reportable.
///
/// ## FP fix: branch-sensitive bare `super` forwarding (2026-04-20)
///
/// Bare `super` implicitly references the enclosing method arguments, but
/// nitrocop had recorded that as a plain "last assignment wins" reference.
/// In branchy rewrites like `case field.type; when :integer then value =
/// value.to_i; when :float then value = value.to_f; end; super`, that kept
/// only the final sibling assignment alive and falsely flagged the others.
/// VariableForce now feeds bare-`super` argument references through the same
/// branch-aware reference walk as normal local reads, matching RuboCop.
///
/// ## FP fix: dynamic class/module constant paths (2026-04-20)
///
/// RuboCop treats `module foo::Baz` and `class foo::Baz` as reads of the
/// outer-scope local `foo`. VariableForce was opening the new class/module
/// scope without first visiting the `constant_path`, so these receiver reads
/// were skipped and nitrocop falsely flagged the initializer. Fixed by
/// visiting the constant path before entering the hard class/module scope.
///
/// ## FP fix: modifier while/until post-condition reads (2026-04-25)
///
/// Ruby statement modifiers like `foo = bar while foo != baz` and
/// `foo = bar until foo` are post-condition loops, so the body assignment
/// feeds the next predicate check. VariableForce still walks modifier
/// `while`/`until` predicates before their bodies, which means a predicate
/// read of a local first introduced by the body is ignored as "undefined" and
/// the body write looks dead. Match RuboCop by suppressing only those body
/// writes whose enclosing post-condition loop predicate reads the same local.
///
/// ## FN fix: rescue-modifier fallback writes are only live with a later read (2026-04-26)
///
/// The rescue-modifier compatibility filter used to suppress every same-name
/// write in a file once it saw `expr rescue name = fallback`. That hid real
/// offenses like `error(..., abort = true)` when another rescue modifier in
/// the method also assigned `abort`, and it hid the rescue-side write in
/// `m_over_c = expr rescue m_over_c = 0`. The filter is now candidate-aware:
/// it only keeps fallback writes and their initializers when a later read can
/// consume the fallback value, and never for fallback writes nested under an
/// outer assignment to the same local.
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
        let post_condition_loop_body_offsets =
            collect_post_condition_loop_body_write_offsets(parse_result);
        let retry_protected_rescue_offsets =
            collect_retry_protected_rescue_capture_offsets(parse_result);
        let mut rescue_modifier_collector = RescueModifierWriteCollector::default();
        rescue_modifier_collector.visit(&parse_result.node());
        let rescue_modifier_writes = rescue_modifier_collector.writes;
        let chained_assignment_descendants =
            collect_chained_assignment_descendant_offsets(parse_result);
        let mut candidates = collector.take_candidates();
        candidates.sort_by_key(|candidate| candidate.node_offset);
        let mut suppressed_chained_descendants: HashSet<usize> = HashSet::new();

        for candidate in candidates {
            if pattern_match_offsets.contains(&candidate.node_offset) {
                continue;
            }

            if suppressed_chained_descendants.contains(&candidate.node_offset) {
                continue;
            }

            let emit = if candidate.captured_protection {
                false
            } else if !candidate.engine_used
                && conditional_operator_offsets.contains(&candidate.node_offset)
            {
                true
            } else if candidate.engine_used {
                false
            } else {
                !should_suppress_multi_rescue_false_positive(&candidate, &rescue_contexts)
                    && !or_condition_offsets.contains(&candidate.node_offset)
                    && !post_condition_loop_body_offsets.contains(&candidate.node_offset)
                    && !should_suppress_rescue_modifier_false_positive(
                        &candidate,
                        &rescue_modifier_writes,
                    )
                    && !retry_protected_rescue_offsets.contains(&candidate.node_offset)
            };

            if !emit {
                continue;
            }

            if let Some(descendants) = chained_assignment_descendants.get(&candidate.node_offset) {
                suppressed_chained_descendants.extend(descendants);
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
    /// Whether the assignment's value flows out via a block capture rather
    /// than a real reference. Used to suppress force-emit pathways
    /// (e.g. the `cond ? var += ... : var` operator collector) that mirror
    /// RuboCop's flagging — RuboCop honours `captured_by_block` via
    /// `Assignment#used?`, so capture-only liveness should also win there.
    captured_protection: bool,
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
                let captured_protection =
                    !assignment.referenced && variable.captured_by_block && !assignment.reassigned;
                candidates.push(AssignmentCandidate {
                    name: variable.name.clone(),
                    node_offset: assignment.node_offset,
                    branch_id: assignment.branch_id,
                    engine_used: assignment.used(variable.captured_by_block),
                    captured_protection,
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
// assigned in the RHS of an `or`/`||` node AND the variable is read after the
// OR — without a later read, both writes are genuinely useless and RuboCop
// reports both (e.g. `(e = 0) == 0 || include?(e = 0)` with `e` never used).

fn collect_or_condition_write_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> HashSet<usize> {
    let mut read_collector = LvarReadOffsetCollector::default();
    read_collector.visit(&parse_result.node());
    let mut collector = OrConditionWriteCollector {
        offsets: HashSet::new(),
        reads_by_name: read_collector.reads_by_name,
    };
    collector.visit(&parse_result.node());
    collector.offsets
}

struct OrConditionWriteCollector {
    offsets: HashSet<usize>,
    reads_by_name: HashMap<Vec<u8>, Vec<usize>>,
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

#[derive(Default)]
struct LvarReadOffsetCollector {
    reads_by_name: HashMap<Vec<u8>, Vec<usize>>,
}

impl<'pr> Visit<'pr> for LvarReadOffsetCollector {
    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        self.reads_by_name
            .entry(node.name().as_slice().to_vec())
            .or_default()
            .push(node.location().start_offset());
    }
}

impl OrConditionWriteCollector {
    fn process_or_node(&mut self, node: &ruby_prism::OrNode<'_>) {
        let or_end = node.location().end_offset();
        let mut lhs_collector = LvarWriteSubtreeCollector::default();
        lhs_collector.visit(&node.left());
        let mut rhs_collector = LvarWriteSubtreeCollector::default();
        rhs_collector.visit(&node.right());

        // For each variable written in LHS that is also written in RHS,
        // suppress the LHS write offset — but only when the variable is read
        // after the OR. If the variable is never read after the OR, both
        // writes are dead and should be reported.
        for (lhs_name, lhs_offset) in &lhs_collector.writes {
            let rhs_writes_same = rhs_collector
                .writes
                .iter()
                .any(|(rhs_name, _)| rhs_name == lhs_name);
            if !rhs_writes_same {
                continue;
            }
            let read_after_or = self
                .reads_by_name
                .get(lhs_name)
                .is_some_and(|offsets| offsets.iter().any(|&o| o >= or_end));
            if read_after_or {
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
// FP suppression: assignments inside post-condition while/until loops
// ---------------------------------------------------------------------------
//
// `begin ... end until cond` and `foo = bar while foo` both execute the body
// before checking the condition. The VF engine may flag a write inside the
// body as unused if the only read is in the loop condition, because it
// processes modifier `while`/`until` conditions before their bodies.

fn collect_post_condition_loop_body_write_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> HashSet<usize> {
    let mut collector = PostConditionLoopBodyWriteCollector::default();
    collector.visit(&parse_result.node());
    collector.offsets
}

#[derive(Default)]
struct PostConditionLoopBodyWriteCollector {
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

impl PostConditionLoopBodyWriteCollector {
    fn process_loop(
        &mut self,
        predicate: &ruby_prism::Node<'_>,
        body: Option<ruby_prism::StatementsNode<'_>>,
        is_post_condition_loop: bool,
    ) {
        if !is_post_condition_loop {
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

impl<'pr> Visit<'pr> for PostConditionLoopBodyWriteCollector {
    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        self.process_loop(
            &node.predicate(),
            node.statements(),
            node.is_begin_modifier()
                || (node.do_keyword_loc().is_none() && node.closing_loc().is_none()),
        );
        ruby_prism::visit_until_node(self, node);
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        self.process_loop(
            &node.predicate(),
            node.statements(),
            node.is_begin_modifier()
                || (node.do_keyword_loc().is_none() && node.closing_loc().is_none()),
        );
        ruby_prism::visit_while_node(self, node);
    }
}

// ---------------------------------------------------------------------------
// FP suppression: live assignments inside rescue modifier expressions
// ---------------------------------------------------------------------------
//
// `x = Float(y) rescue err = "bad"` creates an implicit fallback branch, but
// VF sees `err = "bad"` as a sequential write. Suppress those fallback writes
// and their initializers only when a later read can consume the fallback value.
// Do not suppress `x = foo rescue x = fallback`: RuboCop reports the nested
// fallback assignment even when the outer `x = ...` is read later.

#[derive(Debug, Clone)]
struct RescueModifierWriteInfo {
    name: Vec<u8>,
    rescue_end: usize,
    suppressible: bool,
}

#[derive(Default)]
struct RescueModifierWriteCollector {
    writes: HashMap<usize, RescueModifierWriteInfo>,
    rescue_end_stack: Vec<usize>,
    outer_rescue_assignment_stack: Vec<Vec<u8>>,
}

impl<'pr> Visit<'pr> for RescueModifierWriteCollector {
    fn visit_rescue_modifier_node(&mut self, node: &ruby_prism::RescueModifierNode<'pr>) {
        // Visit the expression (normal path) normally.
        self.visit(&node.expression());

        // Visit the rescue value (fallback path) with its containing rescue
        // range available for later-read checks.
        self.rescue_end_stack.push(node.location().end_offset());
        self.visit(&node.rescue_expression());
        self.rescue_end_stack.pop();
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        let name = node.name().as_slice().to_vec();
        let outer_rescue_assignment = node.value().as_rescue_modifier_node().is_some();
        if outer_rescue_assignment {
            self.outer_rescue_assignment_stack.push(name.clone());
        }

        if let Some(&rescue_end) = self.rescue_end_stack.last() {
            let same_name_outer = self
                .outer_rescue_assignment_stack
                .last()
                .is_some_and(|outer_name| outer_name == &name);
            self.writes.insert(
                node.location().start_offset(),
                RescueModifierWriteInfo {
                    name: name.clone(),
                    rescue_end,
                    suppressible: !same_name_outer,
                },
            );
        }

        ruby_prism::visit_local_variable_write_node(self, node);

        if outer_rescue_assignment {
            self.outer_rescue_assignment_stack.pop();
        }
    }
}

fn should_suppress_rescue_modifier_false_positive(
    candidate: &AssignmentCandidate,
    rescue_modifier_writes: &HashMap<usize, RescueModifierWriteInfo>,
) -> bool {
    if let Some(info) = rescue_modifier_writes.get(&candidate.node_offset) {
        return info.suppressible && candidate_has_reference_after(candidate, info.rescue_end);
    }

    candidate.assignment_states.iter().any(|assignment| {
        assignment.offset > candidate.node_offset
            && rescue_modifier_writes
                .get(&assignment.offset)
                .is_some_and(|info| {
                    info.suppressible
                        && info.name == candidate.name
                        && candidate_has_reference_after(candidate, info.rescue_end)
                })
    })
}

fn candidate_has_reference_after(candidate: &AssignmentCandidate, offset: usize) -> bool {
    candidate
        .reference_states
        .iter()
        .any(|reference| reference.offset >= offset)
}

// ---------------------------------------------------------------------------
// FP suppression: rescue exception captures protected by a sibling retry-rescue
// ---------------------------------------------------------------------------
//
// RuboCop's `process_rescue` treats a `begin ... rescue ... end` containing
// `retry` as a loop, which (combined with the variable_force walk) keeps any
// `rescue X => e` capture of the same name alive in *sibling* begin blocks —
// e.g. an `if/else` whose two branches both have a rescue capturing `e` and
// only one of which actually reads `e` and retries.
//
// The protection does **not** extend into begin blocks that contain the
// retrying begin (e.g. `def f; begin; do_something do; begin; ...; rescue =>
// e; ...; retry; end; end; rescue Timeout::Error => e; end; end` — the outer
// `Timeout::Error => e` is independent of the inner retry). Mirror that by
// tracking each retry-protected begin's full range and only suppressing
// rescue captures whose enclosing begin does **not** contain the retrying
// begin.

fn collect_retry_protected_rescue_capture_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> HashSet<usize> {
    let mut collector = RetryProtectedRescueCollector::default();
    collector.visit(&parse_result.node());
    let mut suppress_collector = RetryProtectedSuppressCollector {
        protected: &collector.protected,
        begin_stack: Vec::new(),
        scope_stack: Vec::new(),
        offsets: HashSet::new(),
    };
    suppress_collector.visit(&parse_result.node());
    suppress_collector.offsets
}

/// (scope_offset, variable_name) keying retry-protection.
type RetryProtectedKey = (usize, Vec<u8>);
/// (begin_start_offset, begin_end_offset).
type BeginRange = (usize, usize);
/// Map keyed by (scope_offset, variable_name) → list of begin ranges inside
/// that scope whose rescue chain has retry-and-read of the variable.
type RetryProtectedMap = HashMap<RetryProtectedKey, Vec<BeginRange>>;

#[derive(Default)]
struct RetryProtectedRescueCollector {
    /// `(scope_offset, name)` → list of `(begin_start, begin_end)` ranges of
    /// `begin` blocks inside that scope whose rescue chain contains `retry`
    /// and a read of `name`. Suppression is scope-local: a retry-rescue in
    /// a different def does not protect captures in this def.
    protected: RetryProtectedMap,
    begin_stack: Vec<BeginRange>,
    scope_stack: Vec<usize>,
}

impl RetryProtectedRescueCollector {
    fn current_begin(&self) -> Option<(usize, usize)> {
        self.begin_stack.last().copied()
    }

    fn current_scope(&self) -> usize {
        self.scope_stack.last().copied().unwrap_or(0)
    }

    fn enter_scope<F: FnOnce(&mut Self)>(&mut self, offset: usize, f: F) {
        self.scope_stack.push(offset);
        f(self);
        self.scope_stack.pop();
    }
}

fn rescue_clause_capture_name(rescue: &ruby_prism::RescueNode<'_>) -> Option<Vec<u8>> {
    let reference = rescue.reference()?;
    let target = reference.as_local_variable_target_node()?;
    Some(target.name().as_slice().to_vec())
}

fn rescue_clause_capture_offset(rescue: &ruby_prism::RescueNode<'_>) -> Option<usize> {
    let reference = rescue.reference()?;
    let target = reference.as_local_variable_target_node()?;
    Some(target.location().start_offset())
}

fn rescue_clause_body_has_retry_and_read(rescue: &ruby_prism::RescueNode<'_>, name: &[u8]) -> bool {
    let Some(stmts) = rescue.statements() else {
        return false;
    };
    let mut scanner = RetryAndReadScanner {
        name,
        has_retry: false,
        has_read: false,
    };
    for stmt in stmts.body().iter() {
        scanner.visit(&stmt);
    }
    scanner.has_retry && scanner.has_read
}

struct RetryAndReadScanner<'n> {
    name: &'n [u8],
    has_retry: bool,
    has_read: bool,
}

impl<'pr> Visit<'pr> for RetryAndReadScanner<'_> {
    fn visit_retry_node(&mut self, _node: &ruby_prism::RetryNode<'pr>) {
        self.has_retry = true;
    }
    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        if node.name().as_slice() == self.name {
            self.has_read = true;
        }
    }
    // Don't recurse into nested scopes — local var resolution doesn't cross
    // those boundaries for rescue exception captures.
    fn visit_def_node(&mut self, _: &ruby_prism::DefNode<'pr>) {}
    fn visit_class_node(&mut self, _: &ruby_prism::ClassNode<'pr>) {}
    fn visit_module_node(&mut self, _: &ruby_prism::ModuleNode<'pr>) {}
    fn visit_lambda_node(&mut self, _: &ruby_prism::LambdaNode<'pr>) {}
}

impl<'pr> Visit<'pr> for RetryProtectedRescueCollector {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let offset = node.location().start_offset();
        self.enter_scope(offset, |this| ruby_prism::visit_def_node(this, node));
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        let offset = node.location().start_offset();
        self.enter_scope(offset, |this| ruby_prism::visit_class_node(this, node));
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        let offset = node.location().start_offset();
        self.enter_scope(offset, |this| ruby_prism::visit_module_node(this, node));
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        let offset = node.location().start_offset();
        self.enter_scope(offset, |this| ruby_prism::visit_lambda_node(this, node));
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        let loc = node.location();
        self.begin_stack
            .push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_begin_node(self, node);
        self.begin_stack.pop();
    }

    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        if let Some(name) = rescue_clause_capture_name(node) {
            if rescue_clause_body_has_retry_and_read(node, &name) {
                if let Some(begin) = self.current_begin() {
                    self.protected
                        .entry((self.current_scope(), name))
                        .or_default()
                        .push(begin);
                }
            }
        }
        ruby_prism::visit_rescue_node(self, node);
    }
}

struct RetryProtectedSuppressCollector<'a> {
    protected: &'a RetryProtectedMap,
    begin_stack: Vec<BeginRange>,
    scope_stack: Vec<usize>,
    offsets: HashSet<usize>,
}

impl<'a> RetryProtectedSuppressCollector<'a> {
    fn current_begin(&self) -> Option<(usize, usize)> {
        self.begin_stack.last().copied()
    }

    fn current_scope(&self) -> usize {
        self.scope_stack.last().copied().unwrap_or(0)
    }

    fn enter_scope<F: FnOnce(&mut Self)>(&mut self, offset: usize, f: F) {
        self.scope_stack.push(offset);
        f(self);
        self.scope_stack.pop();
    }
}

impl<'pr> Visit<'pr> for RetryProtectedSuppressCollector<'_> {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let offset = node.location().start_offset();
        self.enter_scope(offset, |this| ruby_prism::visit_def_node(this, node));
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        let offset = node.location().start_offset();
        self.enter_scope(offset, |this| ruby_prism::visit_class_node(this, node));
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        let offset = node.location().start_offset();
        self.enter_scope(offset, |this| ruby_prism::visit_module_node(this, node));
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        let offset = node.location().start_offset();
        self.enter_scope(offset, |this| ruby_prism::visit_lambda_node(this, node));
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        let loc = node.location();
        self.begin_stack
            .push((loc.start_offset(), loc.end_offset()));
        ruby_prism::visit_begin_node(self, node);
        self.begin_stack.pop();
    }

    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        if let Some(name) = rescue_clause_capture_name(node) {
            if let (Some((b_start, b_end)), Some(ranges)) = (
                self.current_begin(),
                self.protected.get(&(self.current_scope(), name)),
            ) {
                let protected = ranges.iter().any(|&(p_start, p_end)| {
                    // Self-protection: same begin block.
                    (p_start == b_start && p_end == b_end)
                        // Sibling/unrelated: protecting begin is *outside* the
                        // current begin (not nested inside it). RuboCop keeps
                        // a sibling retry-rescue's captures alive but not those
                        // in an outer begin that *contains* the retrying begin.
                        || p_start < b_start
                        || p_end > b_end
                });
                if protected {
                    if let Some(offset) = rescue_clause_capture_offset(node) {
                        self.offsets.insert(offset);
                    }
                }
            }
        }
        ruby_prism::visit_rescue_node(self, node);
    }
}

// ---------------------------------------------------------------------------
// FP suppression: chained assignment (`outer = method(... inner = value ...)`).
// ---------------------------------------------------------------------------
//
// RuboCop's `Lint/UselessAssignment` calls `ignore_node` on chained assignment
// nodes whose RHS is a send after reporting the outer offense. Subsequent
// reverse-iteration over assignments to the *inner* variable then sees the
// inner write as `part_of_ignored_node?` and skips it.
//
// We approximate that with a narrower targeted rule: only suppress lvasgn
// nodes that appear as **direct positional arguments** of the outer call,
// i.e. patterns like `resolve(records, properties, name = value)` where the
// inline assignment is functioning as a self-documenting positional argument.
// This handles the archivesspace case while keeping the suppression scope
// tight enough that Prism's tolerant parsing of malformed source can not
// silently swallow unrelated later statements.

fn collect_chained_assignment_descendant_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
) -> HashMap<usize, HashSet<usize>> {
    let mut collector = ChainedAssignmentDescendantCollector::default();
    collector.visit(&parse_result.node());
    collector.descendants
}

#[derive(Default)]
struct ChainedAssignmentDescendantCollector {
    descendants: HashMap<usize, HashSet<usize>>,
}

impl<'pr> Visit<'pr> for ChainedAssignmentDescendantCollector {
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        let outer_offset = node.location().start_offset();
        let value = node.value();
        if let Some(call) = value.as_call_node() {
            if call.closing_loc().is_some() {
                if let Some(args) = call.arguments() {
                    let mut descendants: HashSet<usize> = HashSet::new();
                    for arg in args.arguments().iter() {
                        if let Some(inner) = arg.as_local_variable_write_node() {
                            descendants.insert(inner.location().start_offset());
                        }
                    }
                    if !descendants.is_empty() {
                        self.descendants.insert(outer_offset, descendants);
                    }
                }
            }
        }
        ruby_prism::visit_local_variable_write_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(UselessAssignment, "cops/lint/useless_assignment");
}
