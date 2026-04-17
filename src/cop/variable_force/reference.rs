/// A single reference (read) of a variable.
#[derive(Debug)]
pub struct Reference {
    /// Byte offset of the reference node in source.
    pub node_offset: usize,
    /// Index into the scope stack at the time the reference was recorded.
    /// Used to determine if the reference crosses a scope boundary.
    pub scope_index: usize,
    /// Whether this is an explicit reference (e.g., `x` in code) vs an
    /// implicit one (e.g., bare `super` implicitly references all method
    /// args, `binding()` implicitly captures all accessible variables).
    pub explicit: bool,
    /// Processing sequence number. References and assignments share a
    /// single counter per engine run, enabling temporal ordering that
    /// reflects actual evaluation order (not byte offset order).
    pub sequence: usize,
    /// Branch context ID, if this reference is inside a conditional branch.
    pub branch_id: Option<usize>,
    /// Full stack of active branch-context IDs from outermost to innermost.
    pub branch_path: Vec<usize>,
}

impl Reference {
    pub fn new(node_offset: usize, scope_index: usize) -> Self {
        Self {
            node_offset,
            scope_index,
            explicit: true,
            sequence: 0,
            branch_id: None,
            branch_path: Vec::new(),
        }
    }

    pub fn implicit(node_offset: usize, scope_index: usize) -> Self {
        Self {
            node_offset,
            scope_index,
            explicit: false,
            sequence: 0,
            branch_id: None,
            branch_path: Vec::new(),
        }
    }
}
