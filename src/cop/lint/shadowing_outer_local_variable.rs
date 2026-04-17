use std::cell::RefCell;
use std::collections::HashSet;

use crate::cop::variable_force::{self, ScopeKind, Variable, VariableTable};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

// Thread-local storage for per-file context data. Within a rayon task, a single
// file is processed sequentially: check_source -> VF engine ->
// before_declaring_variable, so thread-local storage is safe and avoids the
// TOCTOU race that Mutex fields on the shared cop struct would cause.
thread_local! {
    static SHADOWING_CTX: RefCell<ShadowingContext> = RefCell::new(ShadowingContext::new());
}

struct ShadowingContext {
    ractor_block_ranges: Vec<(usize, usize)>,
    begin_node_ranges: Vec<(usize, usize)>,
    branch_intervals: Vec<BranchInterval>,
    expression_ranges: Vec<(usize, usize, usize)>,
    single_stmt_block_bodies: HashSet<usize>,
    inherited_cond_map: Vec<InheritedCondEntry>,
    when_condition_ranges: Vec<(usize, usize, usize)>,
    when_body_ranges: Vec<(usize, usize, usize)>,
    /// (var_name, lhs_offset, rhs_start, rhs_end, rhs_content_hash).
    assignment_rhs_ranges: Vec<(Vec<u8>, usize, usize, usize, u64)>,
    block_body_ranges: Vec<(usize, usize, usize)>,
    defs_local_scope_ranges: Vec<(usize, usize)>,
    singleton_class_body_ranges: Vec<(usize, usize)>,
    /// Variable writes in branch bodies: (var_name, cond_offset, branch_offset, lhs_offset).
    /// Used to detect when a variable is assigned in multiple branches of the
    /// same conditional, which affects RHS-assignment suppression.
    branch_var_writes: Vec<(Vec<u8>, usize, usize, usize)>,
    /// Per-block conditional parent info for replicating RuboCop's check 5/6.
    block_cond_parents: Vec<BlockCondParentEntry>,
}

impl ShadowingContext {
    fn new() -> Self {
        Self {
            ractor_block_ranges: Vec::new(),
            begin_node_ranges: Vec::new(),
            branch_intervals: Vec::new(),
            expression_ranges: Vec::new(),
            single_stmt_block_bodies: HashSet::new(),
            inherited_cond_map: Vec::new(),
            when_condition_ranges: Vec::new(),
            when_body_ranges: Vec::new(),
            assignment_rhs_ranges: Vec::new(),
            block_body_ranges: Vec::new(),
            defs_local_scope_ranges: Vec::new(),
            singleton_class_body_ranges: Vec::new(),
            branch_var_writes: Vec::new(),
            block_cond_parents: Vec::new(),
        }
    }

    /// Look up the conditional branch context for a given byte offset.
    fn branch_info_at(&self, offset: usize) -> VarBranchInfo {
        // Find the innermost (last) interval containing this offset.
        let mut best: Option<&BranchInterval> = None;
        for interval in self.branch_intervals.iter() {
            if interval.start <= offset && offset < interval.end {
                // Pick the innermost (narrowest) interval
                match best {
                    None => best = Some(interval),
                    Some(prev) => {
                        if interval.end - interval.start <= prev.end - prev.start {
                            best = Some(interval);
                        }
                    }
                }
            }
        }

        let when_condition_of_case = self
            .when_condition_ranges
            .iter()
            .find(|(s, e, _)| *s <= offset && offset < *e)
            .map(|(_, _, case_off)| *case_off);

        match best {
            Some(interval) => VarBranchInfo {
                conditional_branch: Some((interval.cond_offset, interval.branch_offset)),
                cond_subsequent_offset: interval.subsequent_offset,
                when_condition_of_case,
                is_condition_var: !interval.is_body,
                is_if_type_cond: interval.is_if_type,
            },
            None => VarBranchInfo {
                conditional_branch: None,
                cond_subsequent_offset: None,
                when_condition_of_case,
                is_condition_var: false,
                is_if_type_cond: false,
            },
        }
    }

    /// Check if a given offset is inside an expression nesting relative to
    /// its enclosing branch interval. Returns true only if the expression
    /// nesting is deeper than the branch entry's expression_depth_base.
    fn is_in_expression_at(&self, offset: usize, branch_expr_depth_base: usize) -> bool {
        self.expression_ranges
            .iter()
            .any(|(s, e, depth)| *s <= offset && offset < *e && *depth > branch_expr_depth_base)
    }

    /// Return the deepest expression nesting that covers `offset`.
    fn max_expression_depth_at(&self, offset: usize) -> usize {
        self.expression_ranges
            .iter()
            .filter(|(s, e, _)| *s <= offset && offset < *e)
            .map(|(_, _, depth)| *depth)
            .max()
            .unwrap_or(0)
    }

    /// Check if a given offset is inside a Ractor.new block.
    fn is_in_ractor_block(&self, offset: usize) -> bool {
        self.ractor_block_ranges
            .iter()
            .any(|(s, e)| *s <= offset && offset < *e)
    }

    fn is_in_begin_wrapper(&self, offset: usize) -> bool {
        self.begin_node_ranges
            .iter()
            .any(|(s, e)| *s <= offset && offset < *e)
    }

    /// Get the innermost branch interval for an offset.
    fn innermost_branch_at(&self, offset: usize) -> Option<BranchInterval> {
        let mut best: Option<&BranchInterval> = None;
        for interval in self.branch_intervals.iter() {
            if interval.start <= offset && offset < interval.end {
                match best {
                    None => best = Some(interval),
                    Some(prev) => {
                        if interval.end - interval.start <= prev.end - prev.start {
                            best = Some(interval);
                        }
                    }
                }
            }
        }
        best.cloned()
    }

    /// Get the inherited conditional branch for an offset (from enclosing block bodies).
    fn inherited_cond_at(&self, offset: usize) -> Option<((usize, usize), bool)> {
        // Find the innermost block body containing this offset
        let mut best: Option<&InheritedCondEntry> = None;
        for entry in self.inherited_cond_map.iter() {
            if entry.block_start <= offset && offset < entry.block_end {
                match best {
                    None => best = Some(entry),
                    Some(prev) => {
                        if entry.block_end - entry.block_start <= prev.block_end - prev.block_start
                        {
                            best = Some(entry);
                        }
                    }
                }
            }
        }
        best.map(|e| (e.cond_branch, e.is_if_type))
    }

    /// Check if an offset is in a when body of a particular case.
    fn in_when_body_of_case_at(&self, offset: usize) -> Option<usize> {
        self.when_body_ranges
            .iter()
            .find(|(s, e, _)| *s <= offset && offset < *e)
            .map(|(_, _, case_off)| *case_off)
    }

    /// Check if there is a multi-statement block/lambda body boundary between
    /// the branch interval and the param offset. Single-statement blocks are
    /// transparent for suppression (matching RuboCop's behavior where
    /// `variable_node.parent` walks up through single-statement blocks).
    /// Multi-statement blocks truly nest the param, so suppression shouldn't apply.
    fn has_multi_stmt_block_boundary_between(
        &self,
        branch_start: usize,
        branch_end: usize,
        param_offset: usize,
    ) -> bool {
        self.block_body_ranges
            .iter()
            .any(|(block_start, body_start, body_end)| {
                *body_start > branch_start
                    && *body_end <= branch_end
                    && *body_start <= param_offset
                    && param_offset < *body_end
                    && !self.single_stmt_block_bodies.contains(block_start)
            })
    }

    /// Check if a block param at `param_offset` is in the RHS of an assignment
    /// whose LHS is at `lhs_offset`. Used to suppress `foo = bar { |foo| }`.
    fn is_in_assignment_rhs(&self, lhs_offset: usize, param_offset: usize) -> bool {
        self.assignment_rhs_ranges
            .iter()
            .any(|(_, lhs, rhs_start, rhs_end, _)| {
                *lhs == lhs_offset && *rhs_start <= param_offset && param_offset < *rhs_end
            })
    }

    /// Check if a block param is in the RHS of a reassignment to the same
    /// variable, where the first declaration is structurally equivalent.
    ///
    /// This replicates a RuboCop quirk: `variable_used_in_declaration_of_outer?`
    /// uses `each_ancestor.any?(declaration_node)` which compares via `==`
    /// (structural equality in parser gem). When `var = expr { |var| }` is
    /// repeated with identical RHS, the lvasgn nodes are structurally equal,
    /// causing suppression even though the block is in a different assignment.
    fn is_in_reassignment_rhs_with_structural_match(
        &self,
        var_name: &[u8],
        outer_offset: usize,
        param_offset: usize,
    ) -> bool {
        // Find which reassignment the param is inside.
        let current_rhs =
            self.assignment_rhs_ranges
                .iter()
                .find(|(name, lhs, rhs_start, rhs_end, _)| {
                    name == var_name
                        && *lhs != outer_offset
                        && *rhs_start <= param_offset
                        && param_offset < *rhs_end
                });
        let Some((_, _, _, _, curr_hash)) = current_rhs else {
            return false;
        };

        // Find the first declaration's RHS.
        let original_rhs = self
            .assignment_rhs_ranges
            .iter()
            .find(|(name, lhs, _, _, _)| name == var_name && *lhs == outer_offset);
        let Some((_, _, orig_rhs_start, orig_rhs_end, orig_hash)) = original_rhs else {
            return false;
        };

        // Check structural equivalence: both RHS must have identical content
        // (same hash) AND the first declaration's RHS must also contain a block.
        // Identical source bytes yield identical parser gem AST nodes, so
        // content hash equality is a sound proxy for structural `==`.
        if orig_hash != curr_hash {
            return false;
        }

        // Verify the first declaration's RHS also contains a block.
        self.block_body_ranges.iter().any(|(block_start, _, _)| {
            *block_start >= *orig_rhs_start && *block_start < *orig_rhs_end
        })
    }

    /// `def self.foo` and `class << self` are modeled as twisted scopes by
    /// VariableForce so receiver expressions can be visited in the outer
    /// scope, but local-variable visibility still stops at the method/class
    /// body. For `defs`, method parameters are part of the inner local scope
    /// even though they are declared before the body starts, so we track a
    /// local-scope span (params + body) instead of only the body range.
    fn is_separated_by_twisted_local_scope(
        &self,
        outer_offset: usize,
        param_offset: usize,
    ) -> bool {
        self.defs_local_scope_ranges
            .iter()
            .chain(self.singleton_class_body_ranges.iter())
            .any(|(start, end)| {
                *start <= param_offset
                    && param_offset < *end
                    && !(*start <= outer_offset && outer_offset < *end)
            })
    }

    /// Check if the block containing `param_offset` is a direct statement-level
    /// child of a conditional branch whose cond_offset matches `outer_cond`,
    /// and the block's position matches RuboCop's check 5 (single-stmt branch)
    /// or check 6 (else clause of if-type conditional).
    fn block_cond_parent_suppresses(&self, param_offset: usize, outer_cond: usize) -> bool {
        // Find the innermost block containing param_offset.
        // block_body_ranges: (block_start, body_start, body_end).
        // Params are between block_start and body_start, body content is
        // between body_start and body_end. We use [block_start, body_end).
        let innermost = self
            .block_body_ranges
            .iter()
            .filter(|(block_start, _, body_end)| {
                *block_start <= param_offset && param_offset < *body_end
            })
            .min_by_key(|(_, _, body_end)| *body_end - param_offset);

        let Some((block_start, _, _)) = innermost else {
            return false;
        };

        if self.is_in_begin_wrapper(*block_start) {
            return false;
        }

        // Check if this specific block has a matching conditional parent entry.
        self.block_cond_parents.iter().any(|entry| {
            entry.block_start == *block_start
                && entry.cond_offset == outer_cond
                && (entry.is_single_stmt_branch || entry.is_else_of_if_type)
        })
    }

    /// Check whether the block param should be suppressed due to conditional
    /// branch context.
    fn should_suppress(&self, outer_info: &VarBranchInfo, param_offset: usize) -> bool {
        let block_interval = self.innermost_branch_at(param_offset);

        let block_branch = block_interval
            .as_ref()
            .map(|i| (i.cond_offset, i.branch_offset));
        let block_is_in_body = block_interval.as_ref().is_some_and(|i| i.is_body);
        let block_single_stmt = block_interval.as_ref().is_some_and(|i| i.single_stmt);
        let is_in_else_clause = block_interval.as_ref().is_some_and(|i| i.is_else_clause);
        let expr_depth_base = block_interval
            .as_ref()
            .map_or(0, |i| i.expression_depth_base);
        let is_nested_in_expression = self.is_in_expression_at(param_offset, expr_depth_base);

        // If the param is inside a multi-statement block body that is nested
        // within the branch interval, the conditional suppression does not
        // apply — the block is truly nested, not a direct child of the branch.
        let has_block_boundary = block_interval.as_ref().is_some_and(|bi| {
            self.has_multi_stmt_block_boundary_between(bi.start, bi.end, param_offset)
        });

        // Check 1: same conditional, different branch
        if let Some(block_branch) = block_branch {
            if !is_nested_in_expression && !has_block_boundary {
                if let Some((outer_cond, outer_branch)) = outer_info.conditional_branch {
                    if outer_cond == block_branch.0 && outer_branch != block_branch.1 {
                        let should_suppress = if outer_info.is_if_type_cond {
                            is_in_else_clause || block_single_stmt
                        } else {
                            block_single_stmt
                        };
                        if should_suppress {
                            return true;
                        }
                    }
                }
            }
        }

        // Check 2: adjacent elsif suppression
        if let Some(block_branch) = block_branch {
            if !is_nested_in_expression
                && !has_block_boundary
                && block_single_stmt
                && (block_is_in_body || !outer_info.is_condition_var)
            {
                if let Some(subsequent_offset) = outer_info.cond_subsequent_offset {
                    if block_branch.0 == subsequent_offset {
                        return true;
                    }
                }
            }
        }

        // Check 3: same conditional node suppression (condition-assigned var)
        if let Some(block_branch) = block_branch {
            if outer_info.is_condition_var
                && block_is_in_body
                && block_single_stmt
                && !is_nested_in_expression
                && !has_block_boundary
            {
                if let Some((outer_cond, outer_branch)) = outer_info.conditional_branch {
                    if outer_cond == block_branch.0 && outer_branch == block_branch.1 {
                        return true;
                    }
                }
            }
        }

        // Check inherited conditional context (from enclosing blocks)
        if block_branch.is_none() {
            if let Some((inherited, is_if_type)) = self.inherited_cond_at(param_offset) {
                if let Some((outer_cond, outer_branch)) = outer_info.conditional_branch {
                    if outer_cond == inherited.0 && outer_branch != inherited.1 && is_if_type {
                        return true;
                    }
                }
            }
        }

        // Check when-condition assignment suppression
        if let (Some(var_case), Some(block_case)) = (
            outer_info.when_condition_of_case,
            self.in_when_body_of_case_at(param_offset),
        ) {
            if var_case == block_case {
                return true;
            }
        }

        // Check block-in-conditional-parent suppression (RuboCop checks 5/6).
        // When a block is a direct statement-level child of a conditional
        // branch body and the outer variable's nearest conditional ancestor
        // matches, suppress if:
        //  - The block's branch is single-statement (check 5: variable_node ==
        //    outer_local_variable_node — block parent is the conditional node)
        //  - The block is in an else clause of an if-type conditional (check 6:
        //    variable_node == outer_local_variable_node.else_branch — block
        //    parent is the else_branch node)
        if let Some((outer_cond, _)) = outer_info.conditional_branch {
            if self.block_cond_parent_suppresses(param_offset, outer_cond) {
                return true;
            }
        }

        // Check: block inside assignment RHS at branch top level (RuboCop Check 6
        // for assignment-wrapped blocks). In parser gem, `d = expr { |d| }` in an
        // else clause has block.parent = lvasgn = if.else_branch, so Check 6 fires.
        // In Prism, the block is inside the call which is inside the assignment, so
        // it's at expression_depth > 0 and the main Check 1 is blocked by
        // is_nested_in_expression. Handle this by checking if the block param is
        // inside an assignment RHS whose LHS is a top-level statement in a
        // different branch of the same conditional as the outer variable.
        if let Some(bi) = block_interval.as_ref() {
            if let Some((outer_cond, outer_branch)) = outer_info.conditional_branch {
                if bi.cond_offset == outer_cond
                    && bi.branch_offset != outer_branch
                    && bi.is_body
                    && bi.is_if_type
                    && bi.single_stmt
                    && !has_block_boundary
                {
                    // Match RuboCop's check 6 only when the block is the
                    // top-level RHS expression. Chained receivers like
                    // `foo = list.select { |foo| }.first` keep the block nested
                    // under another call in parser gem, so RuboCop still flags.
                    let param_expr_depth = self.max_expression_depth_at(param_offset);
                    let in_top_level_assignment_rhs =
                        self.assignment_rhs_ranges
                            .iter()
                            .any(|(_, lhs, rhs_start, rhs_end, _)| {
                                *rhs_start <= param_offset
                                    && param_offset < *rhs_end
                                    && param_expr_depth == bi.expression_depth_base + 1
                                    && bi.start <= *lhs
                                    && *lhs < bi.end
                                    && !self.is_in_expression_at(*lhs, bi.expression_depth_base)
                            });
                    if in_top_level_assignment_rhs {
                        return true;
                    }
                }
            }
        }

        false
    }
}

/// Checks for block parameters or block-local variables that shadow outer local variables.
///
/// ## Root causes of historical FP/FN (corpus conformance ~57%):
///
/// 1. **FP: Variable added to scope before RHS visited.** `visit_local_variable_write_node`
///    called `add_local` before visiting the value child. This caused `foo = bar { |foo| ... }`
///    to incorrectly flag `foo` as shadowing, because the LHS `foo` was already in scope when
///    the block was processed. RuboCop's VariableForce processes the RHS before declaring the
///    variable, so `foo` isn't in scope yet. Fix: visit the value first, then add to scope.
///
/// 2. **FN: Overly aggressive conditional suppression.** The `is_different_conditional_branch`
///    function had a `(None, Some(_)) => true` case that suppressed ALL shadowing when the
///    block was inside any conditional but the outer var was not. Per RuboCop, suppression
///    only applies when BOTH the outer var and the block are in different branches of the
///    SAME conditional node. Fix: remove the incorrect `(None, Some(_))` case.
///
/// 3. **FP: Assignment-RHS suppression blocked in conditional branches.** The
///    `is_in_assignment_rhs` check (for `foo = bar { |foo| }`) was gated behind an
///    `outer_in_branch_body` guard that blocked it whenever the outer variable was
///    inside a conditional branch body. This was too broad: the suppression is valid
///    in branches (e.g. `elsif` body with `ami = items.find { |ami| }`). Fix: the
///    VF engine's `visit_unless_node` was also visiting branches in wrong order
///    (body then else) vs RuboCop (else then body via parser gem's `(if cond B A)`
///    representation). Fixing the VF visit order + removing the over-broad guard
///    makes `is_in_assignment_rhs` work correctly for all branch configurations.
///
/// 4. **FP: `def self...` / `class << self` body leakage.** VariableForce keeps
///    `defs` and singleton-class nodes twisted so receiver expressions stay in the
///    outer scope, but local-variable visibility still stops at the body boundary.
///    Block params inside those bodies were incorrectly seeing outer locals from
///    the enclosing block. The subtlety is that `def self.foo(arg)` method
///    params are inside the method's local scope even though their declaration
///    offsets sit before the body. Fix: track a `defs` local-scope span
///    (params + body) plus singleton-class body ranges, and suppress only when
///    the match crosses those boundaries.
///
/// 5. **FN: conditional-parent propagation crossed multi-statement ancestors.**
///    The check-5/6 compatibility layer for nested blocks should only propagate
///    through ancestor branches while each intermediate branch remains
///    single-statement. Pushing the first multi-statement ancestor wrongly
///    suppressed nested blocks under an outer `else`, such as the corpus
///    `property_schema` example from cenit.
///
/// 6. **FN: assignment-RHS branch suppression was too broad for chained calls.**
///    The Prism-only compensation for `foo = find { |foo| }` in a sibling branch
///    also suppressed chained RHS forms like
///    `section = list.select { |section| }.first`, but RuboCop only suppresses
///    the direct-RHS case. Fix: only apply that compatibility path when the
///    block is exactly one expression layer deep inside the assignment RHS.
///
/// 7. **FN: VariableForce missed outer locals introduced by `for` and `=>`.**
///    Destructured `for` indices and rightward-pattern locals were not being
///    declared in the shared variable engine, so later blocks could not find
///    them as outer locals. Fix: assign all nested `for` targets and
///    `MatchRequiredNode` pattern targets before visiting descendants.
///
/// 8. **FN: check-6 suppression leaked from `unless`/non-`if` branches.**
///    RuboCop's "else branch owns the scope" suppression only applies to real
///    `if`/`elsif` else branches, not `unless` bodies or `case`/`when` bodies.
///    Fix: track that suppression eligibility separately from generic
///    branch-identity metadata and only enable it for true `if`-style else
///    branches.
///
/// 9. **FN: hash literal values looked like direct assignment RHS blocks.**
///    A `proc { |x| }` nested inside a hash assigned in a branch was treated as
///    if it were the direct RHS block of the outer assignment, which suppressed
///    real offenses like Tk's `command: proc { |fnt| ... }` pattern. Fix:
///    record hash literals as expression nesting so only the direct RHS block
///    gets assignment-RHS suppression.
///
/// 10. **FN: explicit `begin` wrappers are not direct conditional children.**
///     RuboCop's check-5/check-6 shortcut only applies when the block's
///     effective variable node is the conditional itself (or its else branch).
///     A `begin ... rescue ... end` wrapper breaks that parentage, so the
///     shortcut must not suppress blocks nested under the begin. Fix: record
///     explicit begin-node ranges and disable that shortcut through them.
///
/// ## Migration to VariableForce
///
/// This cop was migrated from a 1,857-line standalone AST visitor to use the shared
/// VariableForce engine. The cop uses `before_declaring_variable` to detect when a
/// block parameter shadows an outer local variable via `find_variable`. A lightweight
/// `check_source` pass pre-computes two things:
///
/// 1. **Ractor.new block offsets**: Ractor blocks have isolated scope by design;
///    shadowing inside them is intentional and not flagged.
///
/// 2. **Conditional branch context**: Maps byte offsets to their conditional branch
///    context (if/unless/case/when). Used to suppress shadowing when the outer
///    variable and block parameter are in different branches of the same conditional
///    (they can never both be in scope). This includes:
///    - Same-conditional different-branch suppression (Check 1)
///    - Adjacent elsif suppression (Check 2)
///    - Same-conditional-node condition-assignment suppression (Check 3)
///    - When-condition assignment suppression
///    - Inherited conditional context through single-statement block chains
///    - Expression depth tracking for nested-in-expression detection
///    - Per-branch variable write tracking for sibling-branch assignment detection
pub struct ShadowingOuterLocalVariable;

/// A conditional branch interval: all offsets in [start, end) have this context.
#[derive(Clone, Debug)]
struct BranchInterval {
    start: usize,
    end: usize,
    cond_offset: usize,
    branch_offset: usize,
    subsequent_offset: Option<usize>,
    is_body: bool,
    is_if_type: bool,
    single_stmt: bool,
    is_else_clause: bool,
    /// Expression depth base at the point this branch was entered.
    expression_depth_base: usize,
}

/// Inherited conditional context for a block body.
#[derive(Clone, Debug)]
struct InheritedCondEntry {
    /// Start offset of the block body.
    block_start: usize,
    /// End offset of the block body.
    block_end: usize,
    /// The inherited (cond_offset, branch_offset).
    cond_branch: (usize, usize),
    /// Whether the inherited context is from an if-type conditional.
    is_if_type: bool,
}

/// Per-block conditional parent info. Records when a block (or lambda) at
/// statement level (expression_depth == 0) is directly inside a conditional
/// branch body. Used to replicate RuboCop's check 5 (`variable_node ==
/// outer_local_variable_node`) and check 6 (`variable_node ==
/// outer_local_variable_node.else_branch`).
#[derive(Clone, Debug)]
struct BlockCondParentEntry {
    /// The block node's start offset (used as key to match against block_body_ranges).
    block_start: usize,
    /// The conditional's offset (matching the cond_offset in BranchInterval).
    cond_offset: usize,
    /// True if the block's branch is single-statement (check 5 equivalent).
    is_single_stmt_branch: bool,
    /// True if the block is in an else clause of an if-type conditional (check 6 equivalent).
    is_else_of_if_type: bool,
}

/// Info about where a variable was declared, used for suppression checks.
#[derive(Clone, Debug)]
struct VarBranchInfo {
    conditional_branch: Option<(usize, usize)>,
    cond_subsequent_offset: Option<usize>,
    when_condition_of_case: Option<usize>,
    is_condition_var: bool,
    is_if_type_cond: bool,
}

impl ShadowingOuterLocalVariable {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ShadowingOuterLocalVariable {
    fn default() -> Self {
        Self
    }
}

impl Cop for ShadowingOuterLocalVariable {
    fn name(&self) -> &'static str {
        "Lint/ShadowingOuterLocalVariable"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    /// This cop is disabled by default in RuboCop (Enabled: false).
    fn default_enabled(&self) -> bool {
        false
    }

    fn check_source(
        &self,
        _source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        _config: &CopConfig,
        _diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let mut collector = ContextCollector {
            ractor_block_ranges: Vec::new(),
            begin_node_ranges: Vec::new(),
            branch_intervals: Vec::new(),
            expression_ranges: Vec::new(),
            single_stmt_block_bodies: HashSet::new(),
            inherited_cond_map: Vec::new(),
            when_condition_ranges: Vec::new(),
            when_body_ranges: Vec::new(),
            assignment_rhs_ranges: Vec::new(),
            block_body_ranges: Vec::new(),
            defs_local_scope_ranges: Vec::new(),
            singleton_class_body_ranges: Vec::new(),
            branch_var_writes: Vec::new(),
            block_cond_parents: Vec::new(),
            conditional_branch_stack: Vec::new(),
            when_condition_case_offset: None,
            in_when_body_of_case: None,
            expression_depth: 0,
            inherited_cond_branch: None,
        };
        collector.visit(&parse_result.node());

        SHADOWING_CTX.with(|cell| {
            let mut ctx = cell.borrow_mut();
            ctx.ractor_block_ranges = collector.ractor_block_ranges;
            ctx.begin_node_ranges = collector.begin_node_ranges;
            ctx.branch_intervals = collector.branch_intervals;
            ctx.expression_ranges = collector.expression_ranges;
            ctx.single_stmt_block_bodies = collector.single_stmt_block_bodies;
            ctx.inherited_cond_map = collector.inherited_cond_map;
            ctx.when_condition_ranges = collector.when_condition_ranges;
            ctx.when_body_ranges = collector.when_body_ranges;
            ctx.assignment_rhs_ranges = collector.assignment_rhs_ranges;
            ctx.block_body_ranges = collector.block_body_ranges;
            ctx.defs_local_scope_ranges = collector.defs_local_scope_ranges;
            ctx.singleton_class_body_ranges = collector.singleton_class_body_ranges;
            ctx.branch_var_writes = collector.branch_var_writes;
            ctx.block_cond_parents = collector.block_cond_parents;
        });
    }

    fn as_variable_force_consumer(&self) -> Option<&dyn variable_force::VariableForceConsumer> {
        Some(self)
    }
}

impl variable_force::VariableForceConsumer for ShadowingOuterLocalVariable {
    fn before_declaring_variable(
        &self,
        variable: &Variable,
        variable_table: &VariableTable,
        source: &SourceFile,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        // Only check block parameters and block-local variables (shadow args).
        // Method parameters (in def scopes) can't shadow — they're in a hard scope.
        if !variable.is_argument()
            && variable.declaration_kind != variable_force::DeclarationKind::ShadowArg
        {
            return;
        }

        // Skip defs (singleton method) parameters. In Ruby, `def obj.method`
        // creates a hard scope for local variables — method params cannot
        // access outer locals. Our VF models defs as a twisted scope (for
        // receiver evaluation), so find_variable crosses it, but method
        // params in defs should never flag shadowing.
        if variable_table.current_scope().kind == ScopeKind::Defs {
            return;
        }

        let name = &variable.name;

        // Skip underscore-prefixed names
        if name.first() == Some(&b'_') {
            return;
        }

        let param_offset = variable.declaration_offset;
        let outer_offset = variable_table
            .find_variable(name)
            .map(|var| var.declaration_offset);
        let Some(outer_offset) = outer_offset else {
            return;
        };

        // Check suppression conditions using thread-local context data.
        let should_flag = SHADOWING_CTX.with(|cell| {
            let ctx = cell.borrow();

            // Check if we're inside a Ractor.new block — shadowing is intentional
            if ctx.is_in_ractor_block(param_offset) {
                return false;
            }

            if ctx.is_separated_by_twisted_local_scope(outer_offset, param_offset) {
                return false;
            }

            // Look up the outer variable's conditional branch context
            let outer_info = ctx.branch_info_at(outer_offset);

            // Check if the block is in the RHS of the outer variable's assignment.
            // e.g., `foo = bar { |foo| baz(foo) }` — the block is the RHS of foo's
            // assignment, so foo is not yet semantically in scope (RuboCop suppresses
            // via variable_used_in_declaration_of_outer?). This check is purely
            // structural: it only matches when the block param offset falls within
            // the RHS range of the SAME assignment node that declared the outer
            // variable, so it cannot suppress a different assignment or a block
            // in a separate statement.
            if ctx.is_in_assignment_rhs(outer_offset, param_offset) {
                return false;
            }

            // Check for reassignment RHS suppression (RuboCop structural equality
            // quirk). When `var = expr { |var| }` is reassigned with the same pattern,
            // RuboCop's `variable_used_in_declaration_of_outer?` suppresses because
            // the parser gem's `==` considers the lvasgn nodes structurally equal.
            if ctx.is_in_reassignment_rhs_with_structural_match(name, outer_offset, param_offset) {
                return false;
            }

            // Check conditional branch suppression
            if ctx.should_suppress(&outer_info, param_offset) {
                return false;
            }

            true
        });

        if !should_flag {
            return;
        }

        // Adjust offset to include the sigil prefix for sigiled params.
        // RuboCop reports at the full parameter location (including `*`, `**`,
        // `&` sigils) for top-level block params, but at the name only for
        // params inside destructured multi-target (mlhs). The VF engine always
        // stores the name offset. We adjust only when the preceding bytes are
        // the expected sigil AND the param is not inside a destructured context
        // (no `(` between the enclosing `|` and the sigil).
        let src = source.as_bytes();
        let is_destructured = |offset: usize| -> bool {
            // Scan backward from just before the sigil to find `|` or `(`.
            // If we hit `(` before `|`, it's a destructured (mlhs) context.
            for i in (0..offset).rev() {
                match src.get(i) {
                    Some(b'(') => return true,
                    Some(b'|') => return false,
                    _ => {}
                }
            }
            false
        };
        let report_offset = match variable.declaration_kind {
            variable_force::DeclarationKind::RestArg
                if param_offset > 0
                    && src.get(param_offset - 1) == Some(&b'*')
                    && !is_destructured(param_offset - 1) =>
            {
                param_offset - 1
            }
            variable_force::DeclarationKind::KeywordRestArg
                if param_offset > 1
                    && src.get(param_offset - 2) == Some(&b'*')
                    && src.get(param_offset - 1) == Some(&b'*')
                    && !is_destructured(param_offset - 2) =>
            {
                param_offset - 2
            }
            variable_force::DeclarationKind::BlockArg
                if param_offset > 0
                    && src.get(param_offset - 1) == Some(&b'&')
                    && !is_destructured(param_offset - 1) =>
            {
                param_offset - 1
            }
            _ => param_offset,
        };
        let (line, column) = source.offset_to_line_col(report_offset);
        let display_name = String::from_utf8_lossy(name);
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            format!("Shadowing outer local variable - `{display_name}`."),
        ));
    }
}

// ── Context collector (pre-computation visitor) ───────────────────────

/// Entry in the conditional branch stack during context collection.
#[derive(Clone, Copy)]
struct CondBranchEntry {
    cond_offset: usize,
    branch_offset: usize,
    subsequent_offset: Option<usize>,
    is_body: bool,
    is_if_type: bool,
    single_stmt: bool,
    is_else_clause: bool,
    allow_check6_else_suppression: bool,
    expression_depth_base: usize,
}

/// Lightweight AST visitor that pre-computes conditional branch context,
/// Ractor.new block ranges, and expression nesting for the VF hook to query.
struct ContextCollector {
    // Output data
    ractor_block_ranges: Vec<(usize, usize)>,
    begin_node_ranges: Vec<(usize, usize)>,
    branch_intervals: Vec<BranchInterval>,
    expression_ranges: Vec<(usize, usize, usize)>,
    single_stmt_block_bodies: HashSet<usize>,
    inherited_cond_map: Vec<InheritedCondEntry>,
    when_condition_ranges: Vec<(usize, usize, usize)>,
    when_body_ranges: Vec<(usize, usize, usize)>,
    assignment_rhs_ranges: Vec<(Vec<u8>, usize, usize, usize, u64)>,
    block_body_ranges: Vec<(usize, usize, usize)>,
    defs_local_scope_ranges: Vec<(usize, usize)>,
    singleton_class_body_ranges: Vec<(usize, usize)>,
    branch_var_writes: Vec<(Vec<u8>, usize, usize, usize)>,
    block_cond_parents: Vec<BlockCondParentEntry>,

    // Tracking state
    conditional_branch_stack: Vec<CondBranchEntry>,
    when_condition_case_offset: Option<usize>,
    in_when_body_of_case: Option<usize>,
    expression_depth: usize,
    inherited_cond_branch: Option<((usize, usize), bool)>,
}

impl ContextCollector {
    fn push_branch(&mut self, entry: CondBranchEntry, start: usize, end: usize) {
        self.branch_intervals.push(BranchInterval {
            start,
            end,
            cond_offset: entry.cond_offset,
            branch_offset: entry.branch_offset,
            subsequent_offset: entry.subsequent_offset,
            is_body: entry.is_body,
            is_if_type: entry.is_if_type,
            single_stmt: entry.single_stmt,
            is_else_clause: entry.is_else_clause,
            expression_depth_base: entry.expression_depth_base,
        });
        self.conditional_branch_stack.push(entry);
    }

    fn pop_branch(&mut self) {
        self.conditional_branch_stack.pop();
    }

    fn current_cond_branch(&self) -> Option<(usize, usize)> {
        self.conditional_branch_stack
            .last()
            .map(|e| (e.cond_offset, e.branch_offset))
    }

    fn current_is_if_type(&self) -> bool {
        self.conditional_branch_stack
            .last()
            .is_some_and(|e| e.is_if_type)
    }

    /// Record that offsets in [start, end) are inside an expression nesting
    /// at the current expression depth.
    fn record_expression_range(&mut self, start: usize, end: usize) {
        self.expression_ranges
            .push((start, end, self.expression_depth));
    }

    fn visit_if_node_impl(&mut self, node: &ruby_prism::IfNode<'_>) {
        let if_offset = node.location().start_offset();
        let subsequent_offset = node.subsequent().map(|s| s.location().start_offset());

        let then_branch_offset = node
            .statements()
            .map(|s| s.location().start_offset())
            .unwrap_or(if_offset);

        let then_single_stmt = node.statements().is_none_or(|s| s.body().len() <= 1);

        // Visit predicate with then-body conditional context (is_body=false)
        let pred_start = node.predicate().location().start_offset();
        let pred_end = node.predicate().location().end_offset();
        let pred_entry = CondBranchEntry {
            cond_offset: if_offset,
            branch_offset: then_branch_offset,
            subsequent_offset,
            is_body: false,
            is_if_type: true,
            single_stmt: then_single_stmt,
            is_else_clause: false,
            allow_check6_else_suppression: false,
            expression_depth_base: self.expression_depth,
        };
        self.push_branch(pred_entry, pred_start, pred_end);
        self.visit(&node.predicate());
        self.pop_branch();

        // Visit then-body
        if let Some(stmts) = node.statements() {
            let body_start = stmts.location().start_offset();
            let body_end = stmts.location().end_offset();
            let body_entry = CondBranchEntry {
                cond_offset: if_offset,
                branch_offset: then_branch_offset,
                subsequent_offset,
                is_body: true,
                is_if_type: true,
                single_stmt: then_single_stmt,
                is_else_clause: false,
                allow_check6_else_suppression: false,
                expression_depth_base: self.expression_depth,
            };
            self.push_branch(body_entry, body_start, body_end);
            self.visit_statements_node(&stmts);
            self.pop_branch();
        }

        // Visit else/elsif
        if let Some(subsequent) = node.subsequent() {
            if let Some(elsif_node) = subsequent.as_if_node() {
                let branch_offset = subsequent.location().start_offset();
                let sub_start = subsequent.location().start_offset();
                let sub_end = subsequent.location().end_offset();
                let elsif_outer_entry = CondBranchEntry {
                    cond_offset: if_offset,
                    branch_offset,
                    subsequent_offset: None,
                    is_body: true,
                    is_if_type: true,
                    single_stmt: false,
                    is_else_clause: true,
                    allow_check6_else_suppression: true,
                    expression_depth_base: self.expression_depth,
                };
                self.push_branch(elsif_outer_entry, sub_start, sub_end);
                self.visit_if_node_impl(&elsif_node);
                self.pop_branch();
            } else {
                let branch_offset = subsequent.location().start_offset();
                let else_single_stmt = subsequent
                    .as_else_node()
                    .and_then(|e| e.statements())
                    .is_none_or(|s| s.body().len() <= 1);
                let sub_start = subsequent.location().start_offset();
                let sub_end = subsequent.location().end_offset();
                let else_entry = CondBranchEntry {
                    cond_offset: if_offset,
                    branch_offset,
                    subsequent_offset: None,
                    is_body: true,
                    is_if_type: true,
                    single_stmt: else_single_stmt,
                    is_else_clause: true,
                    allow_check6_else_suppression: true,
                    expression_depth_base: self.expression_depth,
                };
                self.push_branch(else_entry, sub_start, sub_end);
                self.visit(&subsequent);
                self.pop_branch();
            }
        }
    }

    fn visit_when_node_with_case_offset(
        &mut self,
        node: &ruby_prism::WhenNode<'_>,
        case_offset: usize,
    ) {
        // Visit when conditions
        let saved = self.when_condition_case_offset;
        self.when_condition_case_offset = Some(case_offset);
        let cond_offset = node.location().start_offset();

        // Record when condition range
        for condition in node.conditions().iter() {
            let start = condition.location().start_offset();
            let end = condition.location().end_offset();
            self.when_condition_ranges.push((start, end, case_offset));

            let cond_entry = CondBranchEntry {
                cond_offset: case_offset,
                branch_offset: cond_offset,
                subsequent_offset: None,
                is_body: false,
                is_if_type: false,
                single_stmt: false,
                is_else_clause: false,
                allow_check6_else_suppression: false,
                expression_depth_base: self.expression_depth,
            };
            self.push_branch(cond_entry, start, end);
            self.visit(&condition);
            self.pop_branch();
        }
        self.when_condition_case_offset = saved;

        // Visit when body
        if let Some(stmts) = node.statements() {
            let saved_body = self.in_when_body_of_case;
            self.in_when_body_of_case = Some(case_offset);
            let body_start = stmts.location().start_offset();
            let body_end = stmts.location().end_offset();
            self.when_body_ranges
                .push((body_start, body_end, case_offset));
            self.visit_statements_node(&stmts);
            self.in_when_body_of_case = saved_body;
        }
    }
}

impl<'pr> Visit<'pr> for ContextCollector {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            self.visit(&receiver);
        }

        if node.receiver().is_some() {
            let scope_start = node
                .parameters()
                .map(|params| params.location().start_offset())
                .or_else(|| node.body().map(|body| body.location().start_offset()));
            if let Some(scope_start) = scope_start {
                self.defs_local_scope_ranges
                    .push((scope_start, node.location().end_offset()));
            }
        }

        // Clear conditional branch context: def creates a hard scope boundary
        // for local variables. Conditional context from enclosing if/unless/case
        // should not leak into the def body, or blocks inside the def would
        // incorrectly suppress shadowing via block_cond_parent_suppresses.
        let saved_cond_stack = std::mem::take(&mut self.conditional_branch_stack);
        let saved_inherited = self.inherited_cond_branch.take();
        if let Some(parameters) = node.parameters() {
            self.visit_parameters_node(&parameters);
        }
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.conditional_branch_stack = saved_cond_stack;
        self.inherited_cond_branch = saved_inherited;
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        if let Some(superclass) = node.superclass() {
            self.visit(&superclass);
        }
        if let Some(body) = node.body() {
            self.visit(&body);
        }
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        if let Some(body) = node.body() {
            self.visit(&body);
        }
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        self.visit(&node.expression());

        if let Some(body) = node.body() {
            self.singleton_class_body_ranges
                .push((body.location().start_offset(), body.location().end_offset()));
            self.visit(&body);
        }
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        self.begin_node_ranges
            .push((node.location().start_offset(), node.location().end_offset()));
        ruby_prism::visit_begin_node(self, node);
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        // Detect Ractor.new blocks
        if is_ractor_new_call(node) {
            if let Some(block) = node.block() {
                if let Some(block_node) = block.as_block_node() {
                    self.ractor_block_ranges.push((
                        block_node.location().start_offset(),
                        block_node.location().end_offset(),
                    ));
                }
            }
            // Visit receiver and arguments normally
            if let Some(receiver) = node.receiver() {
                self.visit(&receiver);
            }
            if let Some(arguments) = node.arguments() {
                self.visit_arguments_node(&arguments);
            }
            if let Some(block) = node.block() {
                if let Some(block_node) = block.as_block_node() {
                    ruby_prism::visit_block_node(self, &block_node);
                }
            }
            return;
        }

        // Visit receiver with expression depth
        if let Some(receiver) = node.receiver() {
            let start = receiver.location().start_offset();
            let end = receiver.location().end_offset();
            self.expression_depth += 1;
            self.record_expression_range(start, end);
            self.visit(&receiver);
            self.expression_depth -= 1;
        }
        if let Some(arguments) = node.arguments() {
            let start = arguments.location().start_offset();
            let end = arguments.location().end_offset();
            self.expression_depth += 1;
            self.record_expression_range(start, end);
            self.visit_arguments_node(&arguments);
            self.expression_depth -= 1;
        }
        if let Some(block) = node.block() {
            self.visit(&block);
        }
    }

    fn visit_hash_node(&mut self, node: &ruby_prism::HashNode<'pr>) {
        let start = node.location().start_offset();
        let end = node.location().end_offset();
        self.expression_depth += 1;
        self.record_expression_range(start, end);
        ruby_prism::visit_hash_node(self, node);
        self.expression_depth -= 1;
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        let var_name = node.name().as_slice().to_vec();
        let lhs_offset = node.location().start_offset();
        let start = node.value().location().start_offset();
        let end = node.value().location().end_offset();
        let rhs_hash = simple_hash(node.value().location().as_slice());
        self.assignment_rhs_ranges
            .push((var_name.clone(), lhs_offset, start, end, rhs_hash));
        // Record branch context for this variable write (used to detect
        // sibling-branch assignments for the same variable).
        if let Some(entry) = self.conditional_branch_stack.last() {
            if entry.is_body {
                let var_name = node.name().as_slice().to_vec();
                self.branch_var_writes.push((
                    var_name,
                    entry.cond_offset,
                    entry.branch_offset,
                    lhs_offset,
                ));
            }
        }
        self.expression_depth += 1;
        self.record_expression_range(start, end);
        self.visit(&node.value());
        self.expression_depth -= 1;
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        let start = node.value().location().start_offset();
        let end = node.value().location().end_offset();
        self.expression_depth += 1;
        self.record_expression_range(start, end);
        self.visit(&node.value());
        self.expression_depth -= 1;
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        let start = node.value().location().start_offset();
        let end = node.value().location().end_offset();
        self.expression_depth += 1;
        self.record_expression_range(start, end);
        self.visit(&node.value());
        self.expression_depth -= 1;
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        let start = node.value().location().start_offset();
        let end = node.value().location().end_offset();
        self.expression_depth += 1;
        self.record_expression_range(start, end);
        self.visit(&node.value());
        self.expression_depth -= 1;
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        let rhs_start = node.value().location().start_offset();
        let rhs_end = node.value().location().end_offset();
        let rhs_hash = simple_hash(node.value().location().as_slice());
        // Record each LHS target's offset as mapping to the RHS range
        for target in node.lefts().iter() {
            if let Some(t) = target.as_local_variable_target_node() {
                self.assignment_rhs_ranges.push((
                    t.name().as_slice().to_vec(),
                    t.location().start_offset(),
                    rhs_start,
                    rhs_end,
                    rhs_hash,
                ));
            }
        }
        if let Some(rest) = node.rest() {
            if let Some(splat) = rest.as_splat_node() {
                if let Some(expr) = splat.expression() {
                    if let Some(t) = expr.as_local_variable_target_node() {
                        self.assignment_rhs_ranges.push((
                            t.name().as_slice().to_vec(),
                            t.location().start_offset(),
                            rhs_start,
                            rhs_end,
                            rhs_hash,
                        ));
                    }
                }
            }
        }
        for target in node.rights().iter() {
            if let Some(t) = target.as_local_variable_target_node() {
                self.assignment_rhs_ranges.push((
                    t.name().as_slice().to_vec(),
                    t.location().start_offset(),
                    rhs_start,
                    rhs_end,
                    rhs_hash,
                ));
            }
        }
        self.expression_depth += 1;
        self.record_expression_range(rhs_start, rhs_end);
        self.visit(&node.value());
        self.expression_depth -= 1;
        // Visit targets (but don't add expression depth)
        for target in node.lefts().iter() {
            self.visit(&target);
        }
        if let Some(rest) = node.rest() {
            self.visit(&rest);
        }
        for target in node.rights().iter() {
            self.visit(&target);
        }
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        let block_body_single_stmt = node
            .body()
            .and_then(|body| body.as_statements_node())
            .is_none_or(|body| body.body().len() <= 1);

        if block_body_single_stmt {
            self.single_stmt_block_bodies
                .insert(node.location().start_offset());
        }

        // Compute inherited conditional context for inner blocks
        let current_cond = self.current_cond_branch();
        let current_if_type = self.current_is_if_type();
        let saved_inherited = self.inherited_cond_branch;
        let new_inherited = if block_body_single_stmt {
            current_cond
                .map(|cb| (cb, current_if_type))
                .or(self.inherited_cond_branch)
        } else {
            None
        };
        self.inherited_cond_branch = new_inherited;

        // Record inherited conditional context for the block body
        if let Some((cond_branch, is_if_type)) = new_inherited {
            if let Some(body) = node.body() {
                self.inherited_cond_map.push(InheritedCondEntry {
                    block_start: body.location().start_offset(),
                    block_end: body.location().end_offset(),
                    cond_branch,
                    is_if_type,
                });
            }
        }

        // Record block body range for block-boundary checks
        if let Some(body) = node.body() {
            self.block_body_ranges.push((
                node.location().start_offset(),
                body.location().start_offset(),
                body.location().end_offset(),
            ));
        }

        // Record conditional parent info for blocks at statement level.
        // This enables RuboCop's check 5/6 suppression for blocks that are
        // direct children of conditional branch bodies.
        //
        // Propagate through single-statement ancestor chains: when a block is
        // the sole statement in a when body, its "variable_node" in RuboCop
        // terms is the case node (one level up). If that case is the sole
        // statement in an else-branch, the variable_node IS the else-branch.
        // Record entries for each ancestor body level as long as the chain
        // remains single-statement (each intermediate branch has ≤1 statement).
        if self.expression_depth == 0 {
            let mut is_innermost = true;
            let mut can_propagate = true;
            for entry in self
                .conditional_branch_stack
                .iter()
                .rev()
                .filter(|e| e.is_body)
            {
                if !is_innermost && (!can_propagate || !entry.single_stmt) {
                    break;
                }
                self.block_cond_parents.push(BlockCondParentEntry {
                    block_start: node.location().start_offset(),
                    cond_offset: entry.cond_offset,
                    // Only the innermost entry gets is_single_stmt_branch = true.
                    // Propagated entries only propagate is_else_of_if_type (RuboCop
                    // Check 6). This prevents over-suppression for case/when where
                    // a block nested inside a single-stmt when body was incorrectly
                    // treated as a direct child of the case node.
                    is_single_stmt_branch: is_innermost && entry.single_stmt,
                    is_else_of_if_type: entry.allow_check6_else_suppression,
                });
                can_propagate = entry.single_stmt;
                is_innermost = false;
            }
        }

        // Clear conditional branch stack for block body
        let saved_cond_stack = std::mem::take(&mut self.conditional_branch_stack);
        let saved_when_body = self.in_when_body_of_case.take();
        ruby_prism::visit_block_node(self, node);
        self.conditional_branch_stack = saved_cond_stack;
        self.in_when_body_of_case = saved_when_body;
        self.inherited_cond_branch = saved_inherited;
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        let lambda_body_single_stmt = node
            .body()
            .and_then(|body| body.as_statements_node())
            .is_none_or(|body| body.body().len() <= 1);

        if lambda_body_single_stmt {
            self.single_stmt_block_bodies
                .insert(node.location().start_offset());
        }

        let current_cond = self.current_cond_branch();
        let current_if_type = self.current_is_if_type();
        let saved_inherited = self.inherited_cond_branch;
        let new_inherited = if lambda_body_single_stmt {
            current_cond
                .map(|cb| (cb, current_if_type))
                .or(self.inherited_cond_branch)
        } else {
            None
        };
        self.inherited_cond_branch = new_inherited;

        if let Some((cond_branch, is_if_type)) = new_inherited {
            if let Some(body) = node.body() {
                self.inherited_cond_map.push(InheritedCondEntry {
                    block_start: body.location().start_offset(),
                    block_end: body.location().end_offset(),
                    cond_branch,
                    is_if_type,
                });
            }
        }

        // Record lambda body range for block-boundary checks
        if let Some(body) = node.body() {
            self.block_body_ranges.push((
                node.location().start_offset(),
                body.location().start_offset(),
                body.location().end_offset(),
            ));
        }

        // Record conditional parent info for lambdas at statement level.
        // Same propagation logic as visit_block_node.
        if self.expression_depth == 0 {
            let mut is_innermost = true;
            let mut can_propagate = true;
            for entry in self
                .conditional_branch_stack
                .iter()
                .rev()
                .filter(|e| e.is_body)
            {
                if !is_innermost && (!can_propagate || !entry.single_stmt) {
                    break;
                }
                self.block_cond_parents.push(BlockCondParentEntry {
                    block_start: node.location().start_offset(),
                    cond_offset: entry.cond_offset,
                    is_single_stmt_branch: is_innermost && entry.single_stmt,
                    is_else_of_if_type: entry.allow_check6_else_suppression,
                });
                can_propagate = entry.single_stmt;
                is_innermost = false;
            }
        }

        let saved_cond_stack = std::mem::take(&mut self.conditional_branch_stack);
        let saved_when_body = self.in_when_body_of_case.take();
        ruby_prism::visit_lambda_node(self, node);
        self.conditional_branch_stack = saved_cond_stack;
        self.in_when_body_of_case = saved_when_body;
        self.inherited_cond_branch = saved_inherited;
    }

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        self.visit_if_node_impl(node);
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        let unless_offset = node.location().start_offset();
        let body_offset = node.statements().map(|s| s.location().start_offset());

        let body_single_stmt = node.statements().is_none_or(|s| s.body().len() <= 1);

        // Visit predicate normally
        self.visit(&node.predicate());

        // Visit else clause FIRST (Parser gem's then-body).
        if let Some(else_clause) = node.else_clause() {
            let branch_offset = else_clause.location().start_offset();
            let else_start = else_clause.location().start_offset();
            let else_end = else_clause.location().end_offset();
            let else_single_stmt = else_clause.statements().is_none_or(|s| s.body().len() <= 1);
            let else_entry = CondBranchEntry {
                cond_offset: unless_offset,
                branch_offset,
                subsequent_offset: body_offset,
                is_body: true,
                is_if_type: true,
                single_stmt: else_single_stmt,
                is_else_clause: false,
                allow_check6_else_suppression: false,
                expression_depth_base: self.expression_depth,
            };
            self.push_branch(else_entry, else_start, else_end);
            self.visit_else_node(&else_clause);
            self.pop_branch();
        }

        // Visit body SECOND (Parser gem's else).
        if let Some(stmts) = node.statements() {
            let branch_offset = stmts.location().start_offset();
            let body_start = stmts.location().start_offset();
            let body_end = stmts.location().end_offset();
            let body_entry = CondBranchEntry {
                cond_offset: unless_offset,
                branch_offset,
                subsequent_offset: None,
                is_body: true,
                is_if_type: true,
                single_stmt: body_single_stmt,
                is_else_clause: true,
                allow_check6_else_suppression: false,
                expression_depth_base: self.expression_depth,
            };
            self.push_branch(body_entry, body_start, body_end);
            self.visit_statements_node(&stmts);
            self.pop_branch();
        }
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        let while_offset = node.location().start_offset();
        let pred_offset = node.predicate().location().start_offset();
        let start = node.location().start_offset();
        let end = node.location().end_offset();
        let entry = CondBranchEntry {
            cond_offset: while_offset,
            branch_offset: pred_offset,
            subsequent_offset: None,
            is_body: true,
            is_if_type: false,
            single_stmt: false,
            is_else_clause: false,
            allow_check6_else_suppression: false,
            expression_depth_base: self.expression_depth,
        };
        self.push_branch(entry, start, end);
        ruby_prism::visit_while_node(self, node);
        self.pop_branch();
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        let until_offset = node.location().start_offset();
        let pred_offset = node.predicate().location().start_offset();
        let start = node.location().start_offset();
        let end = node.location().end_offset();
        let entry = CondBranchEntry {
            cond_offset: until_offset,
            branch_offset: pred_offset,
            subsequent_offset: None,
            is_body: true,
            is_if_type: false,
            single_stmt: false,
            is_else_clause: false,
            allow_check6_else_suppression: false,
            expression_depth_base: self.expression_depth,
        };
        self.push_branch(entry, start, end);
        ruby_prism::visit_until_node(self, node);
        self.pop_branch();
    }

    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'pr>) {
        let case_offset = node.location().start_offset();

        // Visit predicate
        if let Some(pred) = node.predicate() {
            let pred_start = pred.location().start_offset();
            let pred_end = pred.location().end_offset();
            let pred_entry = CondBranchEntry {
                cond_offset: case_offset,
                branch_offset: pred_start,
                subsequent_offset: None,
                is_body: false,
                is_if_type: false,
                single_stmt: true,
                is_else_clause: false,
                allow_check6_else_suppression: false,
                expression_depth_base: self.expression_depth,
            };
            self.push_branch(pred_entry, pred_start, pred_end);
            self.visit(&pred);
            self.pop_branch();
        }

        // Visit each when clause
        for condition in node.conditions().iter() {
            let branch_offset = condition.location().start_offset();
            let when_start = condition.location().start_offset();
            let when_end = condition.location().end_offset();
            let when_single_stmt = condition
                .as_when_node()
                .and_then(|w| w.statements())
                .is_none_or(|s| s.body().len() <= 1);
            let when_entry = CondBranchEntry {
                cond_offset: case_offset,
                branch_offset,
                subsequent_offset: None,
                is_body: true,
                is_if_type: false,
                single_stmt: when_single_stmt,
                is_else_clause: false,
                allow_check6_else_suppression: false,
                expression_depth_base: self.expression_depth,
            };
            self.push_branch(when_entry, when_start, when_end);
            if let Some(when_node) = condition.as_when_node() {
                self.visit_when_node_with_case_offset(&when_node, case_offset);
            } else {
                self.visit(&condition);
            }
            self.pop_branch();
        }

        // Visit else clause
        if let Some(else_clause) = node.else_clause() {
            let branch_offset = else_clause.location().start_offset();
            let else_start = else_clause.location().start_offset();
            let else_end = else_clause.location().end_offset();
            let else_single_stmt = else_clause.statements().is_none_or(|s| s.body().len() <= 1);
            let else_entry = CondBranchEntry {
                cond_offset: case_offset,
                branch_offset,
                subsequent_offset: None,
                is_body: true,
                is_if_type: false,
                single_stmt: else_single_stmt,
                is_else_clause: true,
                allow_check6_else_suppression: false,
                expression_depth_base: self.expression_depth,
            };
            self.push_branch(else_entry, else_start, else_end);
            self.visit_else_node(&else_clause);
            self.pop_branch();
        }
    }

    fn visit_case_match_node(&mut self, node: &ruby_prism::CaseMatchNode<'pr>) {
        let case_offset = node.location().start_offset();

        // Visit predicate
        if let Some(pred) = node.predicate() {
            self.visit(&pred);
        }

        // Visit each `in` clause as a branch of the case_match conditional.
        // Pattern variables captured in `in` clauses are visible in other
        // branches (Ruby scoping), but RuboCop suppresses shadowing when
        // the outer variable is in a different `in`/`else` branch via
        // same_conditions_node_different_branch? (Check 5).
        for condition in node.conditions().iter() {
            let branch_offset = condition.location().start_offset();
            let in_start = condition.location().start_offset();
            let in_end = condition.location().end_offset();
            let in_single_stmt = condition
                .as_in_node()
                .and_then(|n| n.statements())
                .is_none_or(|s| s.body().len() <= 1);
            let in_entry = CondBranchEntry {
                cond_offset: case_offset,
                branch_offset,
                subsequent_offset: None,
                is_body: true,
                is_if_type: false,
                single_stmt: in_single_stmt,
                is_else_clause: false,
                allow_check6_else_suppression: false,
                expression_depth_base: self.expression_depth,
            };
            self.push_branch(in_entry, in_start, in_end);
            self.visit(&condition);
            self.pop_branch();
        }

        // Visit else clause
        if let Some(else_clause) = node.else_clause() {
            let branch_offset = else_clause.location().start_offset();
            let else_start = else_clause.location().start_offset();
            let else_end = else_clause.location().end_offset();
            let else_single_stmt = else_clause.statements().is_none_or(|s| s.body().len() <= 1);
            let else_entry = CondBranchEntry {
                cond_offset: case_offset,
                branch_offset,
                subsequent_offset: None,
                is_body: true,
                is_if_type: false,
                single_stmt: else_single_stmt,
                is_else_clause: true,
                allow_check6_else_suppression: false,
                expression_depth_base: self.expression_depth,
            };
            self.push_branch(else_entry, else_start, else_end);
            self.visit_else_node(&else_clause);
            self.pop_branch();
        }
    }
}

/// Check if a CallNode is `Ractor.new(...)` or `::Ractor.new(...)`.
/// Simple FNV-1a-style hash for comparing RHS source bytes.
fn simple_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn is_ractor_new_call(node: &ruby_prism::CallNode<'_>) -> bool {
    let name = std::str::from_utf8(node.name().as_slice()).unwrap_or("");
    if name != "new" {
        return false;
    }
    if let Some(receiver) = node.receiver() {
        if let Some(constant) = receiver.as_constant_read_node() {
            let const_name = std::str::from_utf8(constant.name().as_slice()).unwrap_or("");
            return const_name == "Ractor";
        }
        if let Some(path) = receiver.as_constant_path_node() {
            if let Some(child) = path.name() {
                let const_name = std::str::from_utf8(child.as_slice()).unwrap_or("");
                return const_name == "Ractor";
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        ShadowingOuterLocalVariable::new(),
        "cops/lint/shadowing_outer_local_variable"
    );
}
