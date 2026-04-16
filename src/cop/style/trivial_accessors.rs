use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Style/TrivialAccessors — flags trivial reader/writer methods that could use
/// `attr_reader`/`attr_writer`.
///
/// ## Investigation notes (2026-03-23)
///
/// **FN root cause (84 offenses):** Methods inside blocks (`describe`, `context`,
/// `Class.new do...end`, etc.) were not checked because the visitor only recognized
/// `class`/`sclass` scopes. RuboCop's `in_module_or_instance_eval?` walks ancestors
/// and only skips methods in `module` or `instance_eval` blocks — regular blocks are
/// transparent, so methods inside them are checked. Fixed by adding `Block` and
/// `InstanceEval` scope kinds and making blocks transparent when walking the scope
/// stack.
///
/// **FP root cause (5 offenses):**
/// 1. Methods inside `instance_eval` blocks were not skipped (4 FP from activeagents).
///    Fixed by detecting `instance_eval` calls and pushing `InstanceEval` scope.
/// 2. Reader with keyword rest params (`def errors(**_args); @errors; end`) was
///    flagged as trivial (1 FP from trailblazer). Fixed by checking `keyword_rest`
///    in the parameter validation.
///
/// ## Investigation notes (2026-03-27)
///
/// **FN root cause 1 (endless accessors):** `def foo = @foo` and similar endless
/// readers were skipped outright because the cop returned early when
/// `end_keyword_loc().is_none()`. RuboCop still checks endless defs here. Prism
/// exposes endless bodies directly (for example an `InstanceVariableReadNode`)
/// instead of wrapping them in a `StatementsNode`, so the cop now normalizes both
/// body shapes before matching trivial readers/writers.
///
/// **FN root cause 2 (top-level multi-statement defs):** RuboCop only exempts a
/// top-level def when it is the root node (`node.parent.nil?`). nitrocop was
/// skipping every def outside class/block scopes, which missed files like
/// `@foo = 1; def foo; @foo; end` and `obj = Object.new; def obj.foo; @foo; end`.
/// Fixed by exempting only the sole root def in the program body while still
/// checking other top-level defs.
///
/// ## Investigation notes (2026-03-28)
///
/// **FN root cause (57 offenses, nested defs):** `visit_def_node` did not recurse
/// into def bodies, so nested defs (def inside def), singleton classes inside defs,
/// and blocks inside defs were all invisible to the visitor. RuboCop's `on_def`
/// fires for ALL def nodes regardless of nesting, and its `in_module_or_instance_eval?`
/// ancestor walk does not include `:def`/`:defs` in the searched types — defs are
/// scope-transparent. Fixed by recursing into def bodies without pushing any scope.
pub struct TrivialAccessors;

/// Default AllowedMethods from vendor config (to_ary, to_a, to_c, ... to_sym).
const DEFAULT_ALLOWED: &[&[u8]] = &[
    b"to_ary",
    b"to_a",
    b"to_c",
    b"to_enum",
    b"to_h",
    b"to_hash",
    b"to_i",
    b"to_int",
    b"to_io",
    b"to_open",
    b"to_path",
    b"to_proc",
    b"to_r",
    b"to_regexp",
    b"to_str",
    b"to_s",
    b"to_sym",
    b"initialize",
];

/// What kind of enclosing scope we're in.
#[derive(Clone, Copy, PartialEq)]
enum ScopeKind {
    /// Inside a `class` or `class << self`
    Class,
    /// Inside a `module` — trivial accessors are skipped here
    Module,
    /// Inside an `instance_eval` block — trivial accessors are skipped here
    InstanceEval,
    /// Inside a regular block (describe, context, Class.new, etc.) — transparent
    Block,
    /// Top level (not inside any class/module/block)
    TopLevel,
}

impl Cop for TrivialAccessors {
    fn name(&self) -> &'static str {
        "Style/TrivialAccessors"
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
        let exact_name_match = config.get_bool("ExactNameMatch", true);
        let allow_predicates = config.get_bool("AllowPredicates", true);
        let allow_dsl_writers = config.get_bool("AllowDSLWriters", true);
        let ignore_class_methods = config.get_bool("IgnoreClassMethods", false);
        let allowed_methods = config.get_string_array("AllowedMethods");
        let sole_root_def_start = parse_result.node().as_program_node().and_then(|program| {
            let mut body = program.statements().body().iter();
            let first = body.next()?;
            if body.next().is_some() {
                return None;
            }
            first
                .as_def_node()
                .map(|def_node| def_node.def_keyword_loc().start_offset())
        });

        let mut visitor = TrivialAccessorsVisitor {
            cop: self,
            source,
            exact_name_match,
            allow_predicates,
            allow_dsl_writers,
            ignore_class_methods,
            allowed_methods: &allowed_methods,
            scope_stack: vec![ScopeKind::TopLevel],
            sole_root_def_start,
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct TrivialAccessorsVisitor<'a> {
    cop: &'a TrivialAccessors,
    source: &'a SourceFile,
    exact_name_match: bool,
    allow_predicates: bool,
    allow_dsl_writers: bool,
    ignore_class_methods: bool,
    allowed_methods: &'a Option<Vec<String>>,
    scope_stack: Vec<ScopeKind>,
    sole_root_def_start: Option<usize>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> TrivialAccessorsVisitor<'a> {
    /// Walk the scope stack from nearest to farthest, looking for the nearest
    /// relevant scope. Blocks are transparent (keep walking).
    ///
    /// Matches RuboCop's `in_module_or_instance_eval?` + `top_level_node?`:
    /// - Class/SingletonClass → check (return true)
    /// - Module → skip (return false)
    /// - InstanceEval → skip (return false)
    /// - Block / TopLevel → transparent, keep walking
    fn should_check_def(&self) -> bool {
        for scope in self.scope_stack.iter().rev() {
            match scope {
                ScopeKind::Class => return true,
                ScopeKind::Module => return false,
                ScopeKind::InstanceEval => return false,
                ScopeKind::Block | ScopeKind::TopLevel => {}
            }
        }
        true
    }

    fn single_body_node<'pr>(def_node: &ruby_prism::DefNode<'pr>) -> Option<ruby_prism::Node<'pr>> {
        let body = def_node.body()?;
        if let Some(stmts) = body.as_statements_node() {
            let mut body_nodes = stmts.body().iter();
            let first = body_nodes.next()?;
            if body_nodes.next().is_some() {
                return None;
            }
            Some(first)
        } else {
            Some(body)
        }
    }

    fn check_def(&mut self, def_node: &ruby_prism::DefNode<'_>) {
        if self.sole_root_def_start == Some(def_node.def_keyword_loc().start_offset()) {
            return;
        }

        if !self.should_check_def() {
            return;
        }

        // Skip class methods (def self.foo) when IgnoreClassMethods is true
        if self.ignore_class_methods && def_node.receiver().is_some() {
            return;
        }

        let method_name = def_node.name();
        let name_bytes = method_name.as_slice();

        // Check allowed methods (config or defaults), plus always skip `initialize`
        let is_allowed = if let Some(allowed) = self.allowed_methods {
            allowed.iter().any(|m| m.as_bytes() == name_bytes) || name_bytes == b"initialize"
        } else {
            DEFAULT_ALLOWED.contains(&name_bytes)
        };

        if is_allowed {
            return;
        }

        // Get body statements
        let single_stmt = match Self::single_body_node(def_node) {
            Some(node) => node,
            None => return,
        };

        // Check for trivial reader: `def foo; @foo; end`
        if let Some(ivar_read) = single_stmt.as_instance_variable_read_node() {
            let ivar_name = ivar_read.name();
            let ivar_bytes = ivar_name.as_slice();
            if ivar_bytes.len() <= 1 {
                return;
            }
            let ivar_without_at = &ivar_bytes[1..];

            // Skip if method has any parameters (requireds, optionals, rest,
            // keyword rest, block, etc.)
            if let Some(params) = def_node.parameters() {
                if !params.requireds().is_empty()
                    || !params.optionals().is_empty()
                    || params.rest().is_some()
                    || !params.keywords().is_empty()
                    || params.keyword_rest().is_some()
                    || params.block().is_some()
                {
                    return;
                }
            }

            // AllowPredicates: skip `def foo?; @foo; end`
            if self.allow_predicates && name_bytes.ends_with(b"?") {
                return;
            }

            if self.exact_name_match && name_bytes != ivar_without_at {
                return;
            }

            let def_loc = def_node.def_keyword_loc();
            let (line, column) = self.source.offset_to_line_col(def_loc.start_offset());
            self.diagnostics.push(self.cop.diagnostic(
                self.source,
                line,
                column,
                "Use `attr_reader` to define trivial reader methods.".to_string(),
            ));
            return;
        }

        // Check for trivial writer: `def foo=(val); @foo = val; end`
        if let Some(ivar_write) = single_stmt.as_instance_variable_write_node() {
            let ivar_name = ivar_write.name();
            let ivar_bytes = ivar_name.as_slice();
            if ivar_bytes.len() <= 1 {
                return;
            }
            let ivar_without_at = &ivar_bytes[1..];

            let is_setter = name_bytes.ends_with(b"=");

            // AllowDSLWriters: if true, skip non-setter writers
            if self.allow_dsl_writers && !is_setter {
                return;
            }

            if is_setter {
                let name_without_eq = &name_bytes[..name_bytes.len() - 1];
                if self.exact_name_match && name_without_eq != ivar_without_at {
                    return;
                }
            } else if self.exact_name_match && name_bytes != ivar_without_at {
                return;
            }

            // Check that the value being assigned is the parameter
            if let Some(params) = def_node.parameters() {
                let requireds: Vec<_> = params.requireds().into_iter().collect();
                if requireds.len() == 1 {
                    let value = ivar_write.value();
                    if value.as_local_variable_read_node().is_none() {
                        return;
                    }
                } else {
                    return;
                }
            } else {
                return;
            }

            let def_loc = def_node.def_keyword_loc();
            let (line, column) = self.source.offset_to_line_col(def_loc.start_offset());
            let msg = "Use `attr_writer` to define trivial writer methods.";
            self.diagnostics.push(
                self.cop
                    .diagnostic(self.source, line, column, msg.to_string()),
            );
        }
    }
}

impl<'pr> Visit<'pr> for TrivialAccessorsVisitor<'_> {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        self.scope_stack.push(ScopeKind::Class);
        // Visit children
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.scope_stack.pop();
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        self.scope_stack.push(ScopeKind::Class);
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.scope_stack.pop();
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        self.scope_stack.push(ScopeKind::Module);
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.scope_stack.pop();
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        // Detect instance_eval blocks: `something.instance_eval do ... end`
        if let Some(block) = node.block() {
            let method_name = node.name();
            if method_name.as_slice() == b"instance_eval" {
                self.scope_stack.push(ScopeKind::InstanceEval);
                if let Some(block_node) = block.as_block_node() {
                    if let Some(body) = block_node.body() {
                        self.visit(&body);
                    }
                }
                self.scope_stack.pop();
                return;
            }

            // Regular block (describe, context, Class.new, etc.)
            self.scope_stack.push(ScopeKind::Block);
            ruby_prism::visit_call_node(self, node);
            self.scope_stack.pop();
        } else {
            ruby_prism::visit_call_node(self, node);
        }
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        self.check_def(node);
        // Recurse into def bodies so nested defs, classes, and blocks are visited.
        // Defs are scope-transparent, matching RuboCop's ancestor walk which does
        // not include :def/:defs in the searched ancestor types.
        if let Some(body) = node.body() {
            self.visit(&body);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(TrivialAccessors, "cops/style/trivial_accessors");

    #[test]
    fn no_panic_on_bare_at_ivar() {
        // Fuzz crash: Prism produces InstanceVariableReadNode with name :"@"
        // (bare @ with no identifier) on malformed input. Previously panicked
        // on `&ivar_bytes[1..]` when ivar_bytes was only 1 byte.
        let source = b"*o(\n@";
        let _diags = crate::testutil::run_cop_full(&TrivialAccessors, source);
    }
}
