/// Lint/ShadowedArgument: checks for method/block arguments that are reassigned
/// before being used.
///
/// ## Investigation findings
///
/// FP root cause: nitrocop did not check whether the argument was ever referenced
/// (read) in the body. RuboCop's VariableForce checks `argument.referenced?` and
/// skips unreferenced arguments. Without this, `def foo(x); x = 42; end` was
/// flagged even though `x` is never read.
///
/// FN root cause: nitrocop only scanned top-level body statements for assignments.
/// RuboCop does a deep scan of ALL assignments in the scope (including those nested
/// inside conditionals, blocks, and lambdas). When a conditional/block assignment
/// precedes an unconditional one, RuboCop reports at the declaration node (location
/// unknown). Nitrocop missed these patterns entirely because it bailed out on
/// encountering a conditional at the top level.
///
/// Additional FN: shorthand assignments (`||=`, `+=`) should stop the scan (the
/// argument is used) but should not prevent detecting a later unconditional
/// reassignment. The old code returned immediately on shorthand, which could miss
/// a subsequent shadowing write.
///
/// Additional FN: `value = super` was treated as "RHS references arg" because
/// `ForwardingSuperNode` unconditionally counted as a reference. RuboCop's
/// `uses_var?` only matches `(lvar %)`, so bare `super` does NOT count.
/// Split into `node_references_local_explicit` (no super) for RHS checks.
///
/// Additional FN: nested blocks/defs inside outer defs/blocks were never visited
/// because `visit_def_node`/`visit_block_node`/`visit_lambda_node` did not recurse
/// into their bodies. Added explicit recursion after checking parameters.
///
/// FP fix: multi-write `a, b, arg = super` was flagged because
/// `node_references_local_explicit` (used for RHS checks) does not count
/// `ForwardingSuperNode` as a reference. But bare `super` implicitly forwards
/// ALL method arguments, so the param IS used on the RHS. Fixed by checking
/// `node.value().as_forwarding_super_node().is_some()` in `visit_multi_write_node`
/// before falling through to `node_references_local_explicit`.
///
/// Additional FN (5 corpus): Three root causes:
/// 1. `collect_param_names`/`find_param_offset` did not handle `BlockParameterNode`
///    (`&block` params), causing block-pass args to be invisible to the cop entirely.
///    (chefspec FN, seeing_is_believing FN)
/// 2. `AssignmentCollector` did not handle `MultiWriteNode` (parallel/destructuring
///    assignment like `a, b = expr`). `LocalVariableTargetNode` targets inside
///    multi-writes were never collected as assignments. (xiki FN x2, brakeman FN)
/// 3. The `&&` short-circuit case (`char && block = lambda { ... }`) was already
///    handled by default visitor recursion into `AndNode`; the actual blocker was
///    cause #1 (`&block` not collected).
///
/// Additional FN (4 corpus, 2026-03-27): two root causes:
/// 1. `collect_param_names`/`find_param_offset` missed `KeywordRestParameterNode`
///    (`**options`), so shadowing of keyword-rest args was never checked.
/// 2. Prior-reference filtering was too broad: any read before the *reporting*
///    assignment suppressed offenses, including reads that occur only after an
///    earlier shadowing write. RuboCop still reports in those cases (for example,
///    conditional shadowing followed by later unconditional reassignment), so the
///    check now only considers reads before the first non-shorthand assignment that
///    writes the arg without reading it on the RHS.
///
/// FP fix (1 corpus, 2026-04-01): `Kernel#binding` implicitly references all
/// local variables in scope. RuboCop's VariableForce calls `variable.reference!`
/// on every accessible variable when it encounters a bare `binding` call. When
/// `binding` appears before the first shadowing assignment, RuboCop considers the
/// arg as referenced and does not flag it. Added detection of `binding` calls
/// (no receiver, no arguments) in `RefCollector` as implicit references, gated
/// by the `IgnoreImplicitReferences` config like `ForwardingSuperNode`.
///
/// FN fix (1 corpus, 2026-04-17): a def (or block/lambda) nested inside an
/// outer begin/ensure body inherited the outer branch context because
/// `visit_def_node`/`visit_block_node`/`visit_lambda_node` saved and reset
/// `branch_depth` but left `branch_stack` populated with the outer begin
/// branch. When an `&&` short-circuit was pushed inside the def, the
/// `current_shadowing_in_branch` scan walked the stack, found the outer
/// non-short-circuit begin branch, and incorrectly marked the def's own
/// shadowing assignment as conditional (seeing_is_believing compatibility.rb
/// line 15 `char && block = lambda { |c| char }`). Fixed by also saving
/// and clearing `branch_stack` on scope entry so inner scope branch checks
/// only see branches pushed after scope entry.
///
/// FP fix (5 corpus, 2026-04-03): assignments in `case` predicates
/// (e.g., `case value = super`) were treated as unconditional because the
/// VariableForce engine visited the case predicate before incrementing
/// `branch_depth`. RuboCop's `conditional_assignment?` walks parent nodes
/// and considers `case` as conditional, so the assignment is treated as
/// branched and doesn't trigger an offense on its own. Fixed by wrapping
/// case (and case-match) predicates in branch context when they contain
/// local variable writes, matching the existing `if`/`unless` handling.
///
/// ## Migration to VariableForce
///
/// This cop was migrated from a 687-line standalone AST visitor to use the shared
/// VariableForce engine. The cop implements `VariableForceConsumer::before_leaving_scope`
/// to check each argument variable for shadowing assignments that precede any reference.
use crate::cop::variable_force::{self, Scope, Variable, VariableTable};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

pub struct ShadowedArgument;

impl Cop for ShadowedArgument {
    fn name(&self) -> &'static str {
        "Lint/ShadowedArgument"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn as_variable_force_consumer(&self) -> Option<&dyn variable_force::VariableForceConsumer> {
        Some(self)
    }
}

impl variable_force::VariableForceConsumer for ShadowedArgument {
    fn before_leaving_scope(
        &self,
        scope: &Scope,
        _variable_table: &VariableTable,
        source: &SourceFile,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let ignore_implicit = config.get_bool("IgnoreImplicitReferences", false);

        for variable in scope.variables.values() {
            if let Some(diag) = check_variable(self, variable, ignore_implicit, source) {
                diagnostics.push(diag);
            }
        }
    }
}

fn check_variable(
    cop: &ShadowedArgument,
    variable: &Variable,
    ignore_implicit: bool,
    source: &SourceFile,
) -> Option<Diagnostic> {
    // Only check arguments
    if !variable.is_argument() {
        return None;
    }

    // Skip underscore-prefixed args (intentionally unused)
    if variable.should_be_unused() {
        return None;
    }

    // RuboCop: `return unless argument.referenced?`
    // If the argument is never referenced at all, no offense.
    if variable.references.is_empty() && !variable.captured_by_block {
        return None;
    }

    // No assignments means no shadowing
    if variable.assignments.is_empty() {
        return None;
    }

    // Find the first "shadowing" assignment: a non-operator assignment where
    // the RHS doesn't reference the variable (i.e., pure overwrite).
    let first_shadowing = variable
        .assignments
        .iter()
        .find(|a| !a.is_operator() && !a.rhs_references_var);

    let first_shadowing = first_shadowing?;
    let shadowing_offset = first_shadowing.node_offset;

    // Check if any reference occurs before the first shadowing assignment.
    // Uses byte offsets (not sequences) because RHS references get a lower
    // sequence than the assignment they belong to (the engine visits RHS first),
    // but their byte offsets are AFTER the assignment start. Using offsets
    // correctly excludes RHS references like `value = super`.
    //
    // RuboCop's IgnoreImplicitReferences option (confusingly named) means
    // "treat implicit references as always counting" — when enabled, ANY
    // implicit reference (from `super`, `binding`) prevents the offense,
    // regardless of source position. This allows patterns like `foo = super`
    // when the user intentionally shadows args to pass them via zero-arity
    // super. When disabled (default), implicit refs use the normal offset
    // check like explicit refs.
    let has_prior_ref = variable.references.iter().any(|r| {
        if !r.explicit && ignore_implicit {
            // IgnoreImplicitReferences: true — implicit refs always count
            // as "argument was used", bypassing the offset check.
            return true;
        }
        if !r.explicit {
            // IgnoreImplicitReferences: false (default) — implicit refs
            // use the same offset check as explicit refs.
            return r.node_offset <= shadowing_offset;
        }
        r.node_offset <= shadowing_offset
    });

    if has_prior_ref {
        return None;
    }

    // The argument is shadowed before being used.
    // Walk assignments to determine the offense location, mirroring RuboCop's
    // `assignment_without_argument_usage` reduce logic.
    let mut location_known = true;

    for asgn in &variable.assignments {
        // Operator assignments always use the argument — mark location unknown
        if asgn.is_operator() {
            location_known = false;
            continue;
        }

        // If the RHS uses the param, not shadowing
        if asgn.rhs_references_var {
            continue;
        }

        // This is a shadowing-style assignment (non-operator, doesn't use param on RHS).
        if asgn.shadowing_in_branch {
            // Inside a conditional — can't tell if it executes.
            // Mark location as unknown and continue looking.
            location_known = false;
            continue;
        }

        // Unconditional shadowing assignment found.
        let offset = if location_known {
            asgn.node_offset
        } else {
            variable.declaration_offset
        };

        let (line, column) = source.offset_to_line_col(offset);
        let name = String::from_utf8_lossy(&variable.name);
        return Some(cop.diagnostic(
            source,
            line,
            column,
            format!("Argument `{name}` was shadowed by a local variable before it was used."),
        ));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::CopConfig;
    use crate::testutil::{assert_cop_no_offenses_full_with_config, run_cop_full_with_config};

    crate::cop_fixture_tests!(ShadowedArgument, "cops/lint/shadowed_argument");

    fn config_ignore_implicit() -> CopConfig {
        let mut config = CopConfig::default();
        config.options.insert(
            "IgnoreImplicitReferences".to_string(),
            serde_yml::Value::Bool(true),
        );
        config
    }

    #[test]
    fn ignore_implicit_refs_suppresses_super_shadow() {
        // With IgnoreImplicitReferences: true, `value = super` should NOT
        // be flagged because the implicit reference from super counts as
        // "argument was used".
        let source = b"def foo(value)\n  case value = super\n  when :a then 1\n  end\nend\n";
        assert_cop_no_offenses_full_with_config(
            &ShadowedArgument,
            source,
            config_ignore_implicit(),
        );
    }

    #[test]
    fn ignore_implicit_refs_plain_super_before_assign() {
        // With IgnoreImplicitReferences: true, standalone `super` before
        // reassignment should suppress the offense.
        let source = b"def foo(value)\n  super\n  value = 42\n  value\nend\n";
        assert_cop_no_offenses_full_with_config(
            &ShadowedArgument,
            source,
            config_ignore_implicit(),
        );
    }

    #[test]
    fn default_config_still_flags_value_equals_super() {
        // With default config (IgnoreImplicitReferences: false), `value = super`
        // should still be flagged.
        let source = b"def foo(value)\n  value = super\n  value\nend\n";
        let diags = run_cop_full_with_config(&ShadowedArgument, source, CopConfig::default());
        assert_eq!(diags.len(), 1);
    }
}
