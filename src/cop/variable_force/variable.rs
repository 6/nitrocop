use super::assignment::Assignment;
use super::engine::branches_exclusive_in_contexts;
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
    pub fn assign(&mut self, assignment: Assignment) {
        if !self.captured_by_block {
            if let Some(prev) = self.assignments.last() {
                if assignment.branch_id == prev.branch_id {
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
        let ref_branch_id = ref_node.branch_id;
        let ref_offset = ref_node.node_offset;
        let mut consumed_branch_ids: Vec<usize> = Vec::new();

        for assignment in self.assignments.iter_mut().rev() {
            // Skip assignments whose branch we've already processed
            if let Some(a_bid) = assignment.branch_id {
                if consumed_branch_ids.contains(&a_bid) {
                    continue;
                }
            }

            let exclusive = is_exclusive(assignment.branch_id, ref_branch_id, branch_contexts);
            if !exclusive {
                assignment.reference(ref_offset);
            }

            // Stop at the first unbranched assignment or same-branch assignment
            if assignment.branch_id.is_none() || assignment.branch_id == ref_branch_id {
                break;
            }

            if let Some(bid) = assignment.branch_id {
                consumed_branch_ids.push(bid);
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

/// Check if two branch IDs represent exclusive branches (same parent,
/// different child index).
fn is_exclusive(
    a: Option<usize>,
    b: Option<usize>,
    branch_contexts: &[super::engine::BranchContext],
) -> bool {
    branches_exclusive_in_contexts(a, b, branch_contexts)
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
