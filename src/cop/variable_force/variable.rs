use super::assignment::Assignment;
use super::reference::Reference;

/// A declared local variable with its full lifetime state.
///
/// Tracks all assignments and references to this variable within its scope,
/// enabling cops to perform per-assignment liveness analysis.
#[derive(Debug)]
pub struct Variable {
    /// The variable name as bytes (matching Prism's name representation).
    pub name: Vec<u8>,
    /// Byte offset of the declaration node (first assignment or parameter).
    pub declaration_offset: usize,
    /// How this variable was declared.
    pub declaration_kind: DeclarationKind,
    /// Index of the scope this variable belongs to in the scope stack.
    pub scope_index: usize,
    /// All assignments to this variable, in source order.
    pub assignments: Vec<Assignment>,
    /// All references to this variable, in source order.
    pub references: Vec<Reference>,
    /// Whether this variable is captured by a nested block/lambda/proc.
    /// When captured, all assignments are considered "used" because the
    /// block may execute at any time and reference any assignment's value.
    pub captured_by_block: bool,
}

impl Variable {
    pub fn new(
        name: Vec<u8>,
        declaration_offset: usize,
        declaration_kind: DeclarationKind,
        scope_index: usize,
    ) -> Self {
        Self {
            name,
            declaration_offset,
            declaration_kind,
            scope_index,
            assignments: Vec::new(),
            references: Vec::new(),
            captured_by_block: false,
        }
    }

    /// Record a new assignment. Marks the previous assignment as reassigned
    /// only if it is in the same branch (or both are unbranched). Assignments
    /// in exclusive branches (e.g., if-then vs if-else) are NOT marked as
    /// reassigned because only one branch executes.
    pub fn assign(
        &mut self,
        assignment: Assignment,
        branch_contexts: &[super::engine::BranchContext],
    ) {
        let assignment_branch_path =
            relevant_branch_path(&assignment.branch_path, self.scope_index, branch_contexts);
        if !self.captured_by_block {
            if let Some(prev) = self.assignments.last() {
                let prev_branch_path =
                    relevant_branch_path(&prev.branch_path, self.scope_index, branch_contexts);
                if assignment_branch_path == prev_branch_path {
                    let prev_mut = self.assignments.last_mut().unwrap();
                    prev_mut.reassign();
                }
            }
        }
        self.assignments.push(assignment);
    }

    /// Record a reference to this variable. Marks the most recent applicable
    /// assignment as referenced.
    pub fn reference(&mut self, ref_node: Reference) {
        // Mark the most recent assignment as referenced
        if let Some(last_assign) = self.assignments.last_mut() {
            last_assign.reference(ref_node.node_offset);
        }
        self.references.push(ref_node);
    }

    /// Record a reference with branch-awareness. Walks backward through
    /// assignments, referencing each one that is NOT in an exclusive branch
    /// with this reference. Stops at the first unbranched assignment or one
    /// in the same branch (like RuboCop's `Variable#reference!`).
    pub fn reference_with_branches(
        &mut self,
        ref_node: Reference,
        branch_contexts: &[super::engine::BranchContext],
    ) {
        let ref_branch_path =
            relevant_branch_path(&ref_node.branch_path, self.scope_index, branch_contexts);
        let ref_offset = ref_node.node_offset;
        let mut consumed_branch_paths: Vec<Vec<usize>> = Vec::new();

        for assignment in self.assignments.iter_mut().rev() {
            let assignment_branch_path =
                relevant_branch_path(&assignment.branch_path, self.scope_index, branch_contexts);

            if !assignment_branch_path.is_empty()
                && consumed_branch_paths.contains(&assignment_branch_path)
            {
                continue;
            }

            let exclusive = assignment_exclusive_with_reference(
                &assignment_branch_path,
                &ref_branch_path,
                branch_contexts,
            );
            if !exclusive {
                assignment.reference(ref_offset);
            }

            // Stop at the first unbranched assignment or same-branch assignment
            if assignment_branch_path.is_empty() || assignment_branch_path == ref_branch_path {
                break;
            }

            if assignment_branch_path
                .last()
                .and_then(|&id| branch_contexts.get(id))
                .is_some_and(|context| !context.may_run_incompletely)
            {
                consumed_branch_paths.push(assignment_branch_path);
            }
        }
        self.references.push(ref_node);
    }

    /// Whether this variable has been referenced at all.
    pub fn used(&self) -> bool {
        !self.references.is_empty() || self.captured_by_block
    }

    /// Whether this variable is an argument (method param or block param).
    pub fn is_argument(&self) -> bool {
        matches!(
            self.declaration_kind,
            DeclarationKind::RequiredArg
                | DeclarationKind::OptionalArg
                | DeclarationKind::RestArg
                | DeclarationKind::KeywordArg
                | DeclarationKind::OptionalKeywordArg
                | DeclarationKind::KeywordRestArg
                | DeclarationKind::BlockArg
        )
    }

    /// Whether this variable is a method argument (not a block argument).
    pub fn is_method_argument(&self) -> bool {
        // Method arguments are those declared in Def/Defs scopes.
        // This is determined by the scope kind, not the declaration kind.
        // The caller should check the scope kind separately.
        self.is_argument()
    }

    /// Whether this variable is a block-local variable (`|x; local|`).
    pub fn is_block_local(&self) -> bool {
        self.declaration_kind == DeclarationKind::ShadowArg
    }

    /// Whether this variable name starts with underscore (convention for
    /// intentionally unused variables).
    pub fn should_be_unused(&self) -> bool {
        self.name.first() == Some(&b'_')
    }
}

/// Check whether the assignment's branch runs exclusively with the reference's
/// branch, following RuboCop's directed branch walk.
fn assignment_exclusive_with_reference(
    assignment_path: &[usize],
    reference_path: &[usize],
    branch_contexts: &[super::engine::BranchContext],
) -> bool {
    for &assignment_id in assignment_path.iter().rev() {
        let Some(assignment_context) = branch_contexts.get(assignment_id) else {
            continue;
        };

        if assignment_context.may_jump_to_other_branch {
            return false;
        }

        for &reference_id in reference_path.iter().rev() {
            let Some(reference_context) = branch_contexts.get(reference_id) else {
                continue;
            };

            if assignment_context.parent_id != reference_context.parent_id {
                continue;
            }

            if assignment_context.predicate_context || reference_context.predicate_context {
                break;
            }

            return assignment_context.child_index != reference_context.child_index;
        }
    }

    false
}

fn relevant_branch_path(
    branch_path: &[usize],
    variable_scope_index: usize,
    branch_contexts: &[super::engine::BranchContext],
) -> Vec<usize> {
    if branch_contexts.is_empty() {
        return branch_path.to_vec();
    }

    branch_path
        .iter()
        .copied()
        .filter(|&id| {
            branch_contexts
                .get(id)
                .is_some_and(|context| context.scope_index >= variable_scope_index)
        })
        .collect()
}

/// How a variable was first declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    /// First assignment: `x = expr`
    Assignment,
    /// Required argument: `def foo(x)`
    RequiredArg,
    /// Optional argument: `def foo(x = 1)`
    OptionalArg,
    /// Rest argument: `def foo(*x)`
    RestArg,
    /// Keyword argument: `def foo(x:)`
    KeywordArg,
    /// Optional keyword argument: `def foo(x: 1)`
    OptionalKeywordArg,
    /// Keyword rest argument: `def foo(**x)`
    KeywordRestArg,
    /// Block argument: `def foo(&x)`
    BlockArg,
    /// Block-local variable: `foo { |x; shadow| }`
    ShadowArg,
    /// Regexp named capture: `/(?<x>\w+)/ =~ str`
    RegexpCapture,
    /// Pattern match variable: `case x; in y; end`
    PatternMatch,
    /// For-loop index: `for x in collection`
    ForIndex,
}
