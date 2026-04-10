/// A single assignment to a variable.
///
/// Tracks whether this specific assignment's value is referenced before being
/// reassigned, enabling per-assignment liveness analysis (as opposed to
/// per-variable analysis which can't distinguish dead intermediate assignments).
#[derive(Debug)]
pub struct Assignment {
    /// Byte offset of the assignment node in source.
    pub node_offset: usize,
    /// Whether any reference consumes this assignment's value.
    pub referenced: bool,
    /// Whether this assignment was overwritten by a later assignment before
    /// being referenced.
    pub reassigned: bool,
    /// Byte offsets of nodes that reference this assignment's value.
    pub references: Vec<usize>,
    /// What kind of assignment this is.
    pub kind: AssignmentKind,
    /// Whether the RHS of this assignment references the same variable.
    /// For `x = x + 1`, this is true because `x` appears on the RHS.
    /// Set by the engine after processing the RHS but before recording
    /// the assignment.
    pub rhs_references_var: bool,
    /// Processing sequence number. Assignments and references are ordered
    /// by sequence, not byte offset, to reflect actual evaluation order.
    /// (RHS of `x = x + 1` is evaluated before the assignment.)
    pub sequence: usize,
    /// Whether this assignment is inside a conditional branch (if/unless/case,
    /// while/until, rescue, block, lambda). Used by ShadowedArgument to
    /// distinguish conditional from unconditional shadowing.
    pub in_branch: bool,
    /// Branch context ID, if this assignment is inside a conditional branch.
    /// Used together with `BranchContext` in the engine to determine whether
    /// two assignments/references are in mutually exclusive branches.
    pub branch_id: Option<usize>,
    /// Byte offset range of the RHS value node (`start..end`). For
    /// `x = Model.create(name: 'Joe')`, this covers `Model.create(name: 'Joe')`.
    /// For block-wrapped assignments like `x = Model.create { ... }`, this
    /// covers the entire block node (matching RuboCop's `right_assignment_node`
    /// which unwraps to the send node inside the block).
    /// `None` for parameter declarations and other non-value assignments.
    pub value_range: Option<(usize, usize)>,
}

impl Assignment {
    pub fn new(node_offset: usize, kind: AssignmentKind) -> Self {
        Self {
            node_offset,
            referenced: false,
            reassigned: false,
            references: Vec::new(),
            kind,
            rhs_references_var: false,
            sequence: 0,
            in_branch: false,
            branch_id: None,
            value_range: None,
        }
    }

    /// Mark this assignment as referenced by a node at the given offset.
    pub fn reference(&mut self, ref_offset: usize) {
        self.referenced = true;
        self.references.push(ref_offset);
    }

    /// Mark this assignment as reassigned (a later assignment overwrites it).
    /// Only marks as reassigned if not yet referenced — if already referenced,
    /// the value was consumed and the reassignment doesn't make it "dead."
    pub fn reassign(&mut self) {
        if !self.referenced {
            self.reassigned = true;
        }
    }

    /// Whether this assignment's value is used (referenced or captured by block).
    /// The `captured_by_block` parameter comes from the parent Variable.
    ///
    /// Matches RuboCop's logic: `(!reassigned? && captured_by_block?) || referenced`.
    /// When a variable is captured by a block, only non-reassigned assignments
    /// are treated as "used" — assignments that were already overwritten before
    /// the block capture are still dead.
    pub fn used(&self, captured_by_block: bool) -> bool {
        self.referenced || (!self.reassigned && captured_by_block)
    }

    /// Whether this is an operator assignment (`+=`, `-=`, etc.) which reads
    /// the variable before writing.
    pub fn is_operator(&self) -> bool {
        matches!(
            self.kind,
            AssignmentKind::Operator | AssignmentKind::LogicalOr | AssignmentKind::LogicalAnd
        )
    }
}

/// The kind of assignment to a variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentKind {
    /// `x = expr`
    Simple,
    /// `x += expr`, `x -= expr`, `x *= expr`, etc.
    Operator,
    /// `x ||= expr`
    LogicalOr,
    /// `x &&= expr`
    LogicalAnd,
    /// `a, b = expr` (part of a multi-write)
    Multiple,
    /// `*a = expr` (rest/splat in multi-write)
    Rest,
    /// `for x in collection` — the loop index variable.
    For,
    /// `/(?<x>\w+)/ =~ str` — named capture group.
    RegexpCapture,
    /// `rescue => x` — exception variable.
    ExceptionCapture,
}
