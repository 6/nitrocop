use crate::cop::Cop;
use crate::diagnostic::Severity;

/// Checks for `# rubocop:disable` comments that can be removed.
///
/// This cop is special: it requires post-processing knowledge of which cops
/// actually fired offenses, so the detection logic lives in `lint_source_inner`
/// in `src/linter.rs`. This struct exists so the cop name is registered and
/// can be referenced in configuration (enabled/disabled/excluded).
///
/// ## Corpus investigation (2026-03-08)
///
/// FP=19 regressed because moved legacy directives like
/// `# rubocop:disable Style/MethodName` and `# rubocop:disable Metrics/LineLength`
/// stopped suppressing their current cops, so nitrocop started reporting the
/// directives themselves as redundant. Fixed centrally in `parse/directives.rs`
/// by honoring moved legacy names whose short name is unchanged.
///
/// ## Corpus investigation (2026-03-24)
///
/// FN=1102: The conservative approach in `is_directive_redundant` never flags
/// unused directives for enabled cops. Attempted aggressive approach (flag ALL
/// unused directives for known+enabled cops, matching RuboCop). This caused
/// the corpus smoke test to drop to 0% match rate — nitrocop's detection gaps
/// mean many directives that ARE needed (because nitrocop misses the offense
/// RuboCop catches) get incorrectly flagged as redundant. Reverted.
///
/// This FN is structural: it decreases naturally as nitrocop's cop coverage
/// improves. No per-cop fix is feasible without a "perfect cop" allowlist.
///
/// ## Corpus investigation (2026-03-29)
///
/// Two improvements:
///
/// 1. **Unknown cop flagging**: Cops not in registry and not renamed are now
///    flagged with "(unknown cop)" suffix, matching RuboCop. These directives
///    can never suppress anything. Added ~650 new matches.
///
/// 2. **Removed `--only` guard**: The `args.only.is_empty()` guard was removed
///    because `is_directive_redundant()` is conservative enough on its own —
///    it never flags directives for enabled cops that "didn't fire", only
///    disabled/excluded/renamed/unknown cops. This makes `check_cop.py --rerun`
///    work for this cop.
///
/// 3. **Renamed cop guard for `--only` mode**: For renamed cops (e.g.,
///    `Style/MethodName` → `Naming/MethodName`), the function now checks the
///    new-name cop's config state. In `--only` mode, if the new-name cop is
///    enabled, the old-name directive might be suppressing its offenses, so
///    we skip (conservative). In normal mode, `check_and_mark_used()` already
///    handles this correctly.
///
/// ## Corpus investigation (2026-04-01)
///
/// **Run-all-for-redundant mode**: When `--only Lint/RedundantCopDisableDirective`
/// is used, all other enabled cops now also execute. Their diagnostics mark
/// disable directives as "used" (then get discarded). Unused directives for
/// enabled cops that matched the file are flagged as redundant — the cop ran
/// and didn't fire, so the directive truly is unnecessary. This resolved ~457
/// FNs (42% reduction) with 0 new FPs.
///
/// **Self-disable suppression**: Offenses within explicit
/// `# rubocop:disable Lint/RedundantCopDisableDirective` regions are now
/// suppressed, matching RuboCop behavior.
///
/// **Cop denylist**: A small set of cops with known detection gaps vs RuboCop
/// (`REDUNDANT_DISABLE_SKIP_COPS`) are excluded from the aggressive flagging
/// to prevent false positives from nitrocop missing offenses that RuboCop
/// catches.
///
/// **Renamed cop skip-list check**: The `is_directive_redundant` path for
/// renamed cops (e.g., `Metrics/LineLength` → `Layout/LineLength`) now checks
/// whether the new-name cop is in `REDUNDANT_DISABLE_SKIP_COPS`. Previously,
/// directives using old renamed names were unconditionally flagged as redundant
/// in `run_all_for_redundant` mode, even when the new-name cop had known
/// detection gaps. This caused ~51 FPs (mostly `Metrics/LineLength`).
/// Resolved 12+ FPs and 276+ FNs with 0 regressions.
///
/// ## Corpus investigation (2026-04-03)
///
/// **Normal-mode aggressive flagging**: Extended `all_cops_ran` from
/// `--only Lint/RedundantCopDisableDirective` to also cover normal mode
/// (no `--only`/`--except`). In normal mode all enabled cops run, so
/// unused directives for non-skip-list cops are genuinely redundant.
///
/// **Skip list pruning**: Removed 19 of 24 entries from
/// `REDUNDANT_DISABLE_SKIP_COPS` — their corpus match rates reached 100%
/// (detection gaps fixed). Retained 5 cops with real gaps:
/// `Layout/LineLength`, `Layout/MultilineOperationIndentation`,
/// `Lint/UselessAssignment`, `Style/RedundantParentheses`,
/// `Style/SafeNavigation`.
///
/// **Renamed cop defensive check**: Added fallback skip-list check for
/// renamed cops in non-`all_cops_ran` mode (e.g. `--except` runs) to
/// prevent FPs when the new-name cop has detection gaps.
///
/// ## Corpus investigation (2026-04-04)
///
/// Cached corpus results showed that `Lint/UnusedMethodArgument` is now at
/// 100.0% match, so keeping it on the conservative redundant-disable denylist
/// masked clearly-unused directives for abstract `raise NotImplementedError`
/// stubs. `Security/YAMLLoad` remains special: RuboCop only runs it for
/// TargetRubyVersion <= 3.0, so Ruby 3.1+ can safely flag inline directives
/// as redundant even though nitrocop keeps the cop as a compatibility stub.
/// Standalone block disables remain conservative to match RuboCop.
pub struct RedundantCopDisableDirective;

pub(crate) fn allow_redundant_disable_flagging_for_known_gap_cop(
    cop_name: &str,
    target_ruby_version: f64,
    is_inline: bool,
) -> bool {
    match cop_name {
        "Lint/UnusedMethodArgument" => true,
        "Security/YAMLLoad" => is_inline && target_ruby_version >= 3.1,
        _ => false,
    }
}

impl Cop for RedundantCopDisableDirective {
    fn name(&self) -> &'static str {
        "Lint/RedundantCopDisableDirective"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    // This cop is intentionally a no-op in check_lines/check_node/check_source.
    // The actual detection happens in lint_source_inner after all cops have run,
    // where we can determine which disable directives actually suppressed an offense.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cop_name() {
        assert_eq!(
            RedundantCopDisableDirective.name(),
            "Lint/RedundantCopDisableDirective"
        );
    }

    #[test]
    fn default_severity_is_warning() {
        assert_eq!(
            RedundantCopDisableDirective.default_severity(),
            Severity::Warning
        );
    }

    #[test]
    fn unused_method_argument_is_no_longer_forced_to_skip() {
        assert!(allow_redundant_disable_flagging_for_known_gap_cop(
            "Lint/UnusedMethodArgument",
            2.7,
            true,
        ));
    }

    #[test]
    fn line_length_stays_skiplisted() {
        assert!(!allow_redundant_disable_flagging_for_known_gap_cop(
            "Layout/LineLength",
            2.7,
            true,
        ));
    }

    #[test]
    fn yaml_load_only_allows_inline_flagging_on_ruby_3_1_and_newer() {
        assert!(!allow_redundant_disable_flagging_for_known_gap_cop(
            "Security/YAMLLoad",
            3.1,
            false,
        ));
        assert!(!allow_redundant_disable_flagging_for_known_gap_cop(
            "Security/YAMLLoad",
            3.0,
            true,
        ));
        assert!(allow_redundant_disable_flagging_for_known_gap_cop(
            "Security/YAMLLoad",
            3.1,
            true,
        ));
    }

    // Full-pipeline tests for this cop live in tests/integration.rs because
    // they need the complete linter pipeline (all cops running + post-processing).
}
