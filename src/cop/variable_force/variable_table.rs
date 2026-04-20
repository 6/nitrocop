use super::assignment::Assignment;
use super::engine::BranchContext;
use super::reference::Reference;
use super::scope::{Scope, ScopeKind};
use super::variable::{DeclarationKind, Variable};

/// Manages the stack of scopes and variable lookup during AST traversal.
///
/// The scope stack grows as the engine enters nested scopes (def, block, class,
/// etc.) and shrinks as it leaves them. Variable lookup walks the stack from
/// innermost to outermost, stopping at hard scope boundaries.
#[derive(Default)]
pub struct VariableTable {
    scope_stack: Vec<Scope>,
    /// Branch contexts from the engine, used for exclusivity checks.
    pub branch_contexts: Vec<BranchContext>,
}

impl VariableTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a new scope onto the stack.
    pub fn push_scope(&mut self, kind: ScopeKind, start_offset: usize, end_offset: usize) {
        self.scope_stack
            .push(Scope::new(kind, start_offset, end_offset));
    }

    /// Pop the innermost scope from the stack and return it.
    pub fn pop_scope(&mut self) -> Scope {
        self.scope_stack.pop().expect("scope stack underflow")
    }

    /// The current (innermost) scope.
    pub fn current_scope(&self) -> &Scope {
        self.scope_stack.last().expect("no current scope")
    }

    /// The current (innermost) scope, mutably.
    pub fn current_scope_mut(&mut self) -> &mut Scope {
        self.scope_stack.last_mut().expect("no current scope")
    }

    /// The depth of the current scope stack (number of scopes).
    pub fn scope_depth(&self) -> usize {
        self.scope_stack.len()
    }

    /// The index of the current scope in the stack.
    pub fn current_scope_index(&self) -> usize {
        self.scope_stack.len().saturating_sub(1)
    }

    /// Declare a variable in the current scope. If the variable already exists
    /// in the current scope, this is a no-op (the existing variable is used).
    /// Returns whether a new variable was created.
    pub fn declare_variable(
        &mut self,
        name: Vec<u8>,
        declaration_offset: usize,
        kind: DeclarationKind,
    ) -> bool {
        let scope_index = self.current_scope_index();
        let scope = self.current_scope_mut();
        if scope.variables.contains_key(&name) {
            return false;
        }
        scope.variables.insert(
            name.clone(),
            Variable::new(name, declaration_offset, kind, scope_index),
        );
        true
    }

    /// Record an assignment to a variable. If the variable doesn't exist in any
    /// accessible scope, it is declared in the current scope first.
    /// If the variable belongs to a different scope than the current one,
    /// `in_branch` is forced to true (block may not execute).
    pub fn assign_to_variable(&mut self, name: &[u8], mut assignment: Assignment) {
        let current_index = self.current_scope_index();
        let contexts = self.branch_contexts.clone();
        // Check if variable exists in accessible scopes
        if let Some(var) = self.find_variable_mut(name) {
            if var.scope_index != current_index {
                var.captured_by_block = true;
                assignment.in_branch = true;
                assignment.shadowing_in_branch = true;
            }
            var.assign(assignment, &contexts);
        } else {
            // New variable — declare in current scope
            let offset = assignment.node_offset;
            self.declare_variable(name.to_vec(), offset, DeclarationKind::Assignment);
            if let Some(var) = self.current_scope_mut().variables.get_mut(name) {
                var.assign(assignment, &contexts);
            }
        }
    }

    /// Record a reference to a variable. Finds the variable in accessible
    /// scopes and records the reference with branch-awareness.
    pub fn reference_variable(&mut self, name: &[u8], reference: Reference) {
        let current_index = self.current_scope_index();
        // Take branch_contexts out to avoid borrow conflict (we need &mut var
        // and &self.branch_contexts simultaneously).
        let contexts = std::mem::take(&mut self.branch_contexts);
        for scope in self.scope_stack.iter_mut().rev() {
            if let Some(var) = scope.variables.get_mut(name) {
                if var.scope_index != current_index {
                    var.captured_by_block = true;
                }
                var.reference_with_branches(reference, &contexts);
                self.branch_contexts = contexts;
                return;
            }
            if scope.kind.is_hard() {
                self.branch_contexts = contexts;
                return;
            }
        }
        self.branch_contexts = contexts;
        // If variable not found, it's a reference to an undefined variable
        // (e.g., from eval or dynamic scope). Silently ignore.
    }

    /// Record a branch-aware implicit reference to all method arguments from
    /// the nearest enclosing method scope. Used by bare `super`, which
    /// forwards method arguments even when the current scope is a nested block.
    pub fn reference_enclosing_method_arguments(&mut self, reference: Reference) {
        let current_index = self.current_scope_index();
        let contexts = std::mem::take(&mut self.branch_contexts);

        for scope in self.scope_stack.iter_mut().rev() {
            match scope.kind {
                ScopeKind::Block | ScopeKind::SingletonClass => continue,
                ScopeKind::Def | ScopeKind::Defs => {
                    for var in scope.variables.values_mut().filter(|v| v.is_argument()) {
                        if var.scope_index != current_index {
                            var.captured_by_block = true;
                        }
                        var.reference_with_branches(
                            Reference {
                                node_offset: reference.node_offset,
                                scope_index: reference.scope_index,
                                explicit: reference.explicit,
                                sequence: reference.sequence,
                                branch_id: reference.branch_id,
                                branch_path: reference.branch_path.clone(),
                            },
                            &contexts,
                        );
                    }
                    self.branch_contexts = contexts;
                    return;
                }
                _ => {
                    self.branch_contexts = contexts;
                    return;
                }
            }
        }

        self.branch_contexts = contexts;
    }

    /// Find a variable by name, walking the scope stack from innermost to
    /// outermost. Stops at hard scope boundaries (def, class, module).
    ///
    /// Block scopes (twisted) allow looking through to outer scopes.
    pub fn find_variable(&self, name: &[u8]) -> Option<&Variable> {
        for scope in self.scope_stack.iter().rev() {
            if let Some(var) = scope.variables.get(name) {
                return Some(var);
            }
            // Hard scopes block visibility to outer scopes
            if scope.kind.is_hard() {
                return None;
            }
        }
        None
    }

    /// Mutable version of `find_variable`.
    pub fn find_variable_mut(&mut self, name: &[u8]) -> Option<&mut Variable> {
        for scope in self.scope_stack.iter_mut().rev() {
            if let Some(var) = scope.variables.get_mut(name) {
                return Some(var);
            }
            if scope.kind.is_hard() {
                return None;
            }
        }
        None
    }

    /// Whether a variable with the given name exists in any accessible scope.
    pub fn variable_exists(&self, name: &[u8]) -> bool {
        self.find_variable(name).is_some()
    }

    /// All scopes accessible from the current scope, walking the stack from
    /// innermost to outermost and stopping at hard scope boundaries.
    /// Used by cops that need to check variables across closure chains.
    pub fn accessible_scopes(&self) -> Vec<&Scope> {
        let mut result = Vec::new();
        for scope in self.scope_stack.iter().rev() {
            result.push(scope);
            if scope.kind.is_hard() {
                break;
            }
        }
        result
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
        if a_id >= self.branch_contexts.len() || b_id >= self.branch_contexts.len() {
            return false;
        }
        let a_ctx = &self.branch_contexts[a_id];
        let b_ctx = &self.branch_contexts[b_id];
        if a_ctx.predicate_context || b_ctx.predicate_context {
            return false;
        }
        a_ctx.parent_id == b_ctx.parent_id && a_ctx.child_index != b_ctx.child_index
    }

    /// All variables accessible from the current scope (for `binding()`
    /// which implicitly references all accessible variables).
    pub fn accessible_variables_mut(&mut self) -> Vec<&mut Variable> {
        let mut result = Vec::new();
        for scope in self.scope_stack.iter_mut().rev() {
            result.extend(scope.variables.values_mut());
            if scope.kind.is_hard() {
                break;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::variable_force::assignment::AssignmentKind;

    fn decl(table: &mut VariableTable, name: &str, kind: DeclarationKind) {
        table.declare_variable(name.as_bytes().to_vec(), 0, kind);
    }

    fn assign(table: &mut VariableTable, name: &str, offset: usize) {
        table.assign_to_variable(
            name.as_bytes(),
            Assignment::new(offset, AssignmentKind::Simple),
        );
    }

    fn refer(table: &mut VariableTable, name: &str, offset: usize) {
        let si = table.current_scope_index();
        table.reference_variable(name.as_bytes(), Reference::new(offset, si));
    }

    // ── Scope stack basics ─────────────────────────────────────────────

    #[test]
    fn test_push_pop_scope() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        assert_eq!(t.scope_depth(), 1);
        t.push_scope(ScopeKind::Def, 10, 90);
        assert_eq!(t.scope_depth(), 2);
        let popped = t.pop_scope();
        assert_eq!(popped.kind, ScopeKind::Def);
        assert_eq!(t.scope_depth(), 1);
    }

    #[test]
    #[should_panic(expected = "scope stack underflow")]
    fn test_pop_empty_panics() {
        let mut t = VariableTable::new();
        t.pop_scope();
    }

    // ── Variable declaration ───────────────────────────────────────────

    #[test]
    fn test_declare_variable() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        assert!(t.declare_variable(b"x".to_vec(), 5, DeclarationKind::Assignment));
        assert!(t.variable_exists(b"x"));
        assert!(!t.variable_exists(b"y"));
    }

    #[test]
    fn test_declare_duplicate_returns_false() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        assert!(t.declare_variable(b"x".to_vec(), 5, DeclarationKind::Assignment));
        assert!(!t.declare_variable(b"x".to_vec(), 10, DeclarationKind::Assignment));
    }

    // ── Hard scope boundaries ──────────────────────────────────────────

    #[test]
    fn test_def_blocks_outer_variable_access() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        decl(&mut t, "x", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Def, 10, 90);
        // x should NOT be visible inside def
        assert!(!t.variable_exists(b"x"));
        assert!(t.find_variable(b"x").is_none());
        t.pop_scope();

        // x still visible at top level
        assert!(t.variable_exists(b"x"));
    }

    #[test]
    fn test_class_blocks_outer_variable_access() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        decl(&mut t, "x", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Class, 10, 90);
        assert!(!t.variable_exists(b"x"));
        t.pop_scope();
    }

    #[test]
    fn test_module_blocks_outer_variable_access() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        decl(&mut t, "x", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Module, 10, 90);
        assert!(!t.variable_exists(b"x"));
        t.pop_scope();
    }

    // ── Twisted scope (block) allows outer access ──────────────────────

    #[test]
    fn test_block_sees_outer_variable() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::Def, 0, 100);
        decl(&mut t, "x", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Block, 10, 90);
        // x IS visible inside block (twisted scope)
        assert!(t.variable_exists(b"x"));
        assert!(t.find_variable(b"x").is_some());
        t.pop_scope();
    }

    #[test]
    fn test_nested_blocks_see_outer() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::Def, 0, 100);
        decl(&mut t, "x", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Block, 10, 90);
        t.push_scope(ScopeKind::Block, 20, 80);
        // x visible through two nested blocks
        assert!(t.variable_exists(b"x"));
        t.pop_scope();
        t.pop_scope();
    }

    #[test]
    fn test_block_inside_class_does_not_see_pre_class_vars() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 200);
        decl(&mut t, "x", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Class, 10, 190);
        t.push_scope(ScopeKind::Block, 20, 180);
        // x NOT visible — class is a hard boundary
        assert!(!t.variable_exists(b"x"));
        t.pop_scope();
        t.pop_scope();
    }

    // ── Assignment tracking ────────────────────────────────────────────

    #[test]
    fn test_assign_creates_variable_if_missing() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        assign(&mut t, "x", 5);
        assert!(t.variable_exists(b"x"));
        let var = t.find_variable(b"x").unwrap();
        assert_eq!(var.assignments.len(), 1);
    }

    #[test]
    fn test_assign_to_outer_marks_captured() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::Def, 0, 100);
        decl(&mut t, "x", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Block, 10, 90);
        assign(&mut t, "x", 20);
        t.pop_scope();

        let var = t.find_variable(b"x").unwrap();
        assert!(var.captured_by_block);
        assert_eq!(var.assignments.len(), 1);
    }

    #[test]
    fn test_cross_scope_assign_forces_in_branch() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::Def, 0, 100);
        decl(&mut t, "x", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Block, 10, 90);
        assign(&mut t, "x", 20);
        t.pop_scope();

        let var = t.find_variable(b"x").unwrap();
        assert!(var.assignments[0].in_branch);
    }

    // ── Reference tracking ─────────────────────────────────────────────

    #[test]
    fn test_reference_marks_used() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        decl(&mut t, "x", DeclarationKind::Assignment);
        refer(&mut t, "x", 10);
        let var = t.find_variable(b"x").unwrap();
        assert!(var.used());
        assert_eq!(var.references.len(), 1);
    }

    #[test]
    fn test_reference_from_block_marks_captured() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::Def, 0, 100);
        decl(&mut t, "x", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Block, 10, 90);
        refer(&mut t, "x", 20);
        t.pop_scope();

        let var = t.find_variable(b"x").unwrap();
        assert!(var.captured_by_block);
        assert!(var.used());
    }

    #[test]
    fn test_reference_to_unknown_var_is_ignored() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        // Should not panic
        refer(&mut t, "nonexistent", 5);
    }

    // ── accessible_variables_mut ───────────────────────────────────────

    #[test]
    fn test_accessible_variables_stops_at_hard_scope() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 200);
        decl(&mut t, "outer", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Def, 10, 190);
        decl(&mut t, "method_var", DeclarationKind::Assignment);

        t.push_scope(ScopeKind::Block, 20, 180);
        decl(&mut t, "block_var", DeclarationKind::Assignment);

        let vars = t.accessible_variables_mut();
        let names: Vec<String> = vars
            .iter()
            .map(|v| String::from_utf8_lossy(&v.name).to_string())
            .collect();
        // Should see block_var and method_var, but NOT outer (def is hard boundary)
        assert!(names.contains(&"block_var".to_string()));
        assert!(names.contains(&"method_var".to_string()));
        assert!(!names.contains(&"outer".to_string()));
    }

    // ── accessible_scopes ──────────────────────────────────────────────

    #[test]
    fn test_accessible_scopes() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 200);
        t.push_scope(ScopeKind::Def, 10, 190);
        t.push_scope(ScopeKind::Block, 20, 180);
        t.push_scope(ScopeKind::Block, 30, 170);

        let scopes = t.accessible_scopes();
        // Should see: Block(30), Block(20), Def(10) — stops at Def (hard)
        assert_eq!(scopes.len(), 3);
        assert_eq!(scopes[0].kind, ScopeKind::Block);
        assert_eq!(scopes[1].kind, ScopeKind::Block);
        assert_eq!(scopes[2].kind, ScopeKind::Def);
    }

    // ── Multiple assignments to same variable ──────────────────────────

    #[test]
    fn test_reassignment_marks_previous_as_reassigned() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        assign(&mut t, "x", 5);
        assign(&mut t, "x", 15);

        let var = t.find_variable(b"x").unwrap();
        assert_eq!(var.assignments.len(), 2);
        assert!(var.assignments[0].reassigned);
        assert!(!var.assignments[1].reassigned);
    }

    #[test]
    fn test_referenced_assignment_not_marked_reassigned() {
        let mut t = VariableTable::new();
        t.push_scope(ScopeKind::TopLevel, 0, 100);
        assign(&mut t, "x", 5);
        refer(&mut t, "x", 10); // reference before second assignment
        assign(&mut t, "x", 15);

        let var = t.find_variable(b"x").unwrap();
        assert!(var.assignments[0].referenced);
        assert!(!var.assignments[0].reassigned); // was referenced, so not "dead"
    }
}
