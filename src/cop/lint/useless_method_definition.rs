/// Lint/UselessMethodDefinition
///
/// Checks for useless method definitions that just delegate to `super`.
///
/// ## Investigation findings (2026-03-14)
///
/// Root causes of FPs and FNs vs RuboCop:
///
/// 1. **FN: `keyword_rest` (`**kwargs`) incorrectly excluded** — RuboCop's
///    `use_rest_or_optional_args?` only checks `:restarg, :optarg, :kwoptarg`.
///    It does NOT exclude `:kwrestarg` (`**kwargs`). Nitrocop was checking
///    `keyword_rest().is_some()` and returning early, missing methods like
///    `def foo(**opts); super; end`.
///
/// 2. **FP: generic method macro wrappers not detected** — RuboCop skips
///    `method_definition_with_modifier?` which returns true for generic macros
///    like `memoize def foo; super; end` but false for access modifiers like
///    `private def foo; super; end`. Nitrocop had no parent-context check,
///    flagging both equally. Fixed by switching to a `check_source` visitor
///    that can inspect parent CallNode context.
///
/// 3. **FN: `super` with block arg forwarding** — For `def foo(&block);
///    super(&block); end`, RuboCop compares `node.arguments.map(&:source)`
///    which matches `"&block"` == `"&block"`. Nitrocop only checked
///    `LocalVariableReadNode` names, missing `BlockArgumentNode`. Fixed by
///    using source-text comparison for super args matching.
use ruby_prism::Visit;

use crate::cop::shared::access_modifier_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

pub struct UselessMethodDefinition;

impl Cop for UselessMethodDefinition {
    fn name(&self) -> &'static str {
        "Lint/UselessMethodDefinition"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["**/app/controllers/**/*.rb", "**/app/mailers/**/*.rb"]
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let mut visitor = UselessMethodVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            inside_non_access_modifier_call: false,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct UselessMethodVisitor<'a, 'src> {
    cop: &'a UselessMethodDefinition,
    source: &'src SourceFile,
    diagnostics: Vec<Diagnostic>,
    /// True when we are inside a CallNode that is NOT an access modifier
    /// (e.g., `memoize def foo; super; end`). DefNodes in this context should
    /// not be flagged.
    inside_non_access_modifier_call: bool,
}

impl<'pr> Visit<'pr> for UselessMethodVisitor<'_, '_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        // Check if this call wraps a def node as an argument (e.g., `memoize def foo`)
        // and whether it's an access modifier or a generic macro.
        let is_non_access_modifier = if node.receiver().is_none() {
            let name = node.name().as_slice();
            !access_modifier_predicates::is_access_modifier_name(name)
        } else {
            // Has a receiver — not a bare call, treat as generic macro
            true
        };

        if is_non_access_modifier && has_def_argument(node) {
            let prev = self.inside_non_access_modifier_call;
            self.inside_non_access_modifier_call = true;
            ruby_prism::visit_call_node(self, node);
            self.inside_non_access_modifier_call = prev;
        } else {
            ruby_prism::visit_call_node(self, node);
        }
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        self.check_def(node);
        ruby_prism::visit_def_node(self, node);
    }
}

impl UselessMethodVisitor<'_, '_> {
    fn check_def(&mut self, def_node: &ruby_prism::DefNode<'_>) {
        // Skip if wrapped in a non-access-modifier call (e.g., `memoize def foo`)
        if self.inside_non_access_modifier_call {
            return;
        }

        // Skip methods with rest args (*args), optional args (x=1), or optional
        // keyword args (x: 1). These change the calling convention so `super` is
        // not equivalent to removing the method entirely.
        // NOTE: We do NOT skip keyword_rest (**kwargs) — RuboCop doesn't either.
        if let Some(params) = def_node.parameters() {
            if !params.optionals().is_empty()
                || params.rest().is_some()
                || params
                    .keywords()
                    .iter()
                    .any(|k| k.as_optional_keyword_parameter_node().is_some())
            {
                return;
            }
        }

        let body = match def_node.body() {
            Some(b) => b,
            None => return,
        };

        let stmts = match body.as_statements_node() {
            Some(s) => s,
            None => return,
        };

        let body_nodes: Vec<_> = stmts.body().iter().collect();
        if body_nodes.len() != 1 {
            return;
        }

        let first = &body_nodes[0];

        // ForwardingSuperNode is `super` with implicit forwarding (no parens).
        // But skip if it has a block — `super do ... end` adds behavior.
        if let Some(fwd_super) = first.as_forwarding_super_node() {
            if fwd_super.block().is_none() {
                self.report(def_node);
            }
            return;
        }

        // SuperNode is explicit `super(args)` — flag if args match params using
        // source-text comparison (matching RuboCop's approach).
        if let Some(super_node) = first.as_super_node() {
            // Skip if super has a literal block (`super { ... }` / `super do ... end`)
            // — adds behavior. But allow BlockArgumentNode (`super(&block)`) which is
            // just forwarding the block param.
            if let Some(block) = super_node.block() {
                if block.as_block_argument_node().is_none() {
                    return;
                }
            }
            if super_args_match_params_by_source(self.source, def_node.parameters(), &super_node) {
                self.report(def_node);
            }
        }
    }

    fn report(&mut self, def_node: &ruby_prism::DefNode<'_>) {
        let loc = def_node.location();
        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Useless method definition detected. The method just delegates to `super`.".to_string(),
        ));
    }
}

/// Check if a CallNode has any DefNode arguments.
fn has_def_argument(call: &ruby_prism::CallNode<'_>) -> bool {
    if let Some(args) = call.arguments() {
        for arg in args.arguments().iter() {
            if arg.as_def_node().is_some() {
                return true;
            }
        }
    }
    false
}

/// Compare super call arguments against method parameters using source text,
/// matching RuboCop's `node.arguments.map(&:source) == def_node.arguments.map(&:source)`.
///
/// This handles all parameter types uniformly: positional, keyword, block args,
/// keyword splats, etc.
fn super_args_match_params_by_source(
    source: &SourceFile,
    params: Option<ruby_prism::ParametersNode<'_>>,
    super_node: &ruby_prism::SuperNode<'_>,
) -> bool {
    // Collect super arg source texts. In Prism, block arguments (`&block`) are
    // in `block()` not `arguments()`, so we need to include them separately.
    let mut super_arg_sources: Vec<&[u8]> = Vec::new();
    if let Some(args) = super_node.arguments() {
        for arg in args.arguments().iter() {
            let loc = arg.location();
            super_arg_sources.push(&source.as_bytes()[loc.start_offset()..loc.end_offset()]);
        }
    }
    // Include block argument (&block) if present
    if let Some(block) = super_node.block() {
        if block.as_block_argument_node().is_some() {
            let loc = block.location();
            super_arg_sources.push(&source.as_bytes()[loc.start_offset()..loc.end_offset()]);
        }
    }

    let params = match params {
        Some(p) => p,
        None => return super_arg_sources.is_empty(),
    };

    // Collect all parameter source texts in declaration order.
    // RuboCop uses `def_node.arguments` which iterates params in order.
    let mut param_sources: Vec<&[u8]> = Vec::new();

    for req in params.requireds().iter() {
        let loc = req.location();
        param_sources.push(&source.as_bytes()[loc.start_offset()..loc.end_offset()]);
    }

    // Optional args — these are already filtered out above, but be defensive
    for opt in params.optionals().iter() {
        let loc = opt.location();
        param_sources.push(&source.as_bytes()[loc.start_offset()..loc.end_offset()]);
    }

    // Rest param (*args)
    if let Some(rest) = params.rest() {
        let loc = rest.location();
        param_sources.push(&source.as_bytes()[loc.start_offset()..loc.end_offset()]);
    }

    // Post-rest required params
    for post in params.posts().iter() {
        let loc = post.location();
        param_sources.push(&source.as_bytes()[loc.start_offset()..loc.end_offset()]);
    }

    // Required keyword args
    for kw in params.keywords().iter() {
        let loc = kw.location();
        param_sources.push(&source.as_bytes()[loc.start_offset()..loc.end_offset()]);
    }

    // Keyword rest (**kwargs)
    if let Some(kw_rest) = params.keyword_rest() {
        let loc = kw_rest.location();
        param_sources.push(&source.as_bytes()[loc.start_offset()..loc.end_offset()]);
    }

    // Block param (&block)
    if let Some(block) = params.block() {
        let loc = block.location();
        param_sources.push(&source.as_bytes()[loc.start_offset()..loc.end_offset()]);
    }

    if super_arg_sources.len() != param_sources.len() {
        return false;
    }

    // Compare each super arg source text against param source text
    for (arg_src, param_src) in super_arg_sources.iter().zip(param_sources.iter()) {
        if *arg_src != *param_src {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        UselessMethodDefinition,
        "cops/lint/useless_method_definition"
    );
}
