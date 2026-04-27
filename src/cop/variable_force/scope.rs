use std::collections::HashMap;

use super::variable::Variable;

/// Scope kinds matching RuboCop's SCOPE_TYPES and TWISTED_SCOPE_TYPES.
///
/// "Twisted" scopes (Block, Defs, SingletonClass) have child nodes that belong
/// to the OUTER scope (e.g., method call arguments before a block, the receiver
/// of `def self.method`). These children must be processed in the enclosing
/// scope before the twisted scope is pushed.
///
/// "Hard" scopes (Def, Class, Module, TopLevel) completely isolate local
/// variable visibility — outer locals cannot be accessed from within.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// `def method_name` — hard scope boundary.
    Def,
    /// `def self.method_name` / `def obj.method_name` — twisted scope.
    /// The receiver expression belongs to the outer scope.
    Defs,
    /// `block {}`, `do...end`, numbered/it-parameter blocks — twisted scope.
    /// Block parameters are in scope; method call arguments are outer scope.
    Block,
    /// `class Foo` — hard scope boundary.
    /// The superclass expression (`< Base`) belongs to the outer scope.
    Class,
    /// `class << obj` — twisted scope.
    /// The receiver expression belongs to the outer scope.
    SingletonClass,
    /// `module Foo` — hard scope boundary.
    Module,
    /// Top-level program scope.
    TopLevel,
}

impl ScopeKind {
    /// Whether this scope can access local variables from enclosing scopes.
    /// Matches RuboCop's `find_variable` which only walks through block-like
    /// scopes (`block`, `numblock`, `itblock`); `def`/`defs`/class/module/
    /// sclass all act as hard local-variable boundaries.
    pub fn is_twisted(&self) -> bool {
        matches!(self, Self::Block)
    }

    /// Whether this scope creates a hard boundary that blocks access to outer
    /// local variables.
    pub fn is_hard(&self) -> bool {
        !self.is_twisted()
    }
}

/// A scope context for local variable visibility.
///
/// Each scope maintains its own set of declared variables. Variable lookup
/// traverses the scope stack from innermost to outermost, stopping at hard
/// scope boundaries.
pub struct Scope {
    pub kind: ScopeKind,
    /// Byte offset of the scope node's start in the source.
    pub node_start_offset: usize,
    /// Byte offset of the scope node's end in the source.
    pub node_end_offset: usize,
    /// Variables declared in this scope, keyed by name.
    pub variables: HashMap<Vec<u8>, Variable>,
    /// Whether this scope's body is empty (no statements). Set by the engine
    /// for Block scopes; used by `Lint/UnusedBlockArgument` to implement
    /// `IgnoreEmptyBlocks`.
    pub body_empty: bool,
}

impl Scope {
    pub fn new(kind: ScopeKind, start_offset: usize, end_offset: usize) -> Self {
        Self {
            kind,
            node_start_offset: start_offset,
            node_end_offset: end_offset,
            variables: HashMap::new(),
            body_empty: false,
        }
    }
}
