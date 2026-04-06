use crate::cop::shared::node_type::{CALL_NODE, DEF_NODE, SYMBOL_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Naming/PredicatePrefix checks that predicate method names end with `?`
/// and do not start with a forbidden prefix.
///
/// ## Investigation findings (FN=66 fix):
/// - Root cause 1: nitrocop only iterated over ForbiddenPrefixes to identify predicates,
///   but RuboCop iterates over NamePrefix. When a prefix is in NamePrefix but NOT in
///   ForbiddenPrefixes, the method should still be flagged to add `?` suffix.
/// - Root cause 2: `is_attr?` with forbidden prefix `is_` was not flagged. RuboCop
///   computes expected_name by stripping the prefix (→ `attr?`), so method != expected
///   and it gets flagged. Nitrocop returned early because `?` was already present.
/// - Root cause 3: Singleton methods (`def self.is_attr`) were not checked. RuboCop
///   uses `alias on_defs on_def` to check both instance and singleton methods.
/// - Fix: iterate over NamePrefix, compute expected_name per RuboCop logic (strip prefix
///   if in ForbiddenPrefixes, else keep; append `?` if not present), skip if method_name
///   already equals expected_name. Support singleton methods via DefNode receiver check.
///
/// ## Investigation findings (FN=61 fix):
/// - Root cause: all 61 FNs had `# rubocop:disable Naming/PredicateName` inline comments.
///   `Naming/PredicateName` is the old (renamed) name for `Naming/PredicatePrefix`.
///   Nitrocop's directive legacy alias system incorrectly treated same-department renames
///   with different short names as valid aliases. RuboCop does NOT honor the old name in
///   disable comments when the short name changed (its `Registry.qualified_cop_name`
///   resolves by short-name lookup, and `PredicateName` doesn't match `PredicatePrefix`).
/// - Fix: changed `build_directive_legacy_aliases` in `directives.rs` to only include
///   renames where the short name is the same (e.g., `Lint/Eval` → `Security/Eval`),
///   excluding same-department renames with changed short names.
///
/// ## Corpus investigation (2026-03-23) — extended corpus FP=2
///
/// FP=1 (directive): `has_tag?` in mysociety/alaveteli with
/// `# rubocop:disable Naming::PredicateName` (old cop name, `::` separator).
/// RuboCop's `DirectiveComment::COP_NAME_PATTERN` is `([A-Za-z]\w+/)*[A-Za-z]\w+`,
/// which only recognizes `/` as separator — `:` is not `\w`. So for
/// `Naming::PredicateName`, the regex captures only `Naming` (stops at `:`), and
/// RuboCop treats it as a department-level disable covering ALL Naming cops.
/// The earlier FN=61 finding (that RuboCop does NOT honor old short names) was
/// correct for the `/` form (`Naming/PredicateName`), but the `::` form is a
/// different code path — it never reaches `Registry.qualified_cop_name` because
/// only `Naming` is extracted.
/// Fix: changed `normalize_directive_cop_name` to return just the department
/// token (the part before `::`) instead of converting `::` to `/`.
///
/// FP=2 (vendor-path): `is_utf8?` in noosfero/noosfero `vendor/` dir.
/// Systemic vendor-path config/exclusion noise, not a cop bug.
pub struct PredicatePrefix;

impl PredicatePrefix {
    fn check_method_name(
        &self,
        source: &SourceFile,
        name_str: &str,
        name_offset: usize,
        config: &CopConfig,
    ) -> Vec<Diagnostic> {
        // NamePrefix identifies which prefixes mark a method as a predicate.
        // ForbiddenPrefixes is the subset that should be removed.
        let name_prefixes = config
            .get_string_array("NamePrefix")
            .unwrap_or_else(|| vec!["is_".into(), "has_".into(), "have_".into(), "does_".into()]);

        let forbidden_prefixes = config
            .get_string_array("ForbiddenPrefixes")
            .unwrap_or_else(|| vec!["is_".into(), "has_".into(), "have_".into(), "does_".into()]);

        let allowed_methods = config
            .get_string_array("AllowedMethods")
            .unwrap_or_else(|| vec!["is_a?".into()]);

        // UseSorbetSigs: when true, only flag methods with T::Boolean return sigs.
        // We don't support Sorbet sig analysis, so when enabled we skip all checks
        // (conservative: no false positives).
        let use_sorbet_sigs = config.get_bool("UseSorbetSigs", false);
        if use_sorbet_sigs {
            return Vec::new();
        }

        // Setter methods (ending in =) are not predicates
        if name_str.ends_with('=') {
            return Vec::new();
        }

        // Check AllowedMethods
        if allowed_methods.iter().any(|m| m == name_str) {
            return Vec::new();
        }

        // Iterate over NamePrefix (not ForbiddenPrefixes) to identify predicates,
        // matching RuboCop's `predicate_prefixes.each` loop.
        for prefix in &name_prefixes {
            // Check if method starts with this prefix followed by a non-digit
            if !name_str.starts_with(prefix.as_str()) {
                continue;
            }
            let after_prefix = &name_str[prefix.len()..];
            if after_prefix.is_empty() || after_prefix.starts_with(|c: char| c.is_ascii_digit()) {
                continue;
            }

            // Compute expected_name per RuboCop logic:
            // - If prefix is in ForbiddenPrefixes, strip it
            // - Otherwise, keep the name as-is
            // - Append ? if not already present
            let expected = if forbidden_prefixes.iter().any(|fp| fp == prefix) {
                let stripped = &name_str[prefix.len()..];
                if name_str.ends_with('?') {
                    stripped.to_string()
                } else {
                    format!("{stripped}?")
                }
            } else if name_str.ends_with('?') {
                name_str.to_string()
            } else {
                format!("{name_str}?")
            };

            // Skip if method already equals the expected name
            if name_str == expected {
                continue;
            }

            let (line, column) = source.offset_to_line_col(name_offset);

            return vec![self.diagnostic(
                source,
                line,
                column,
                format!("Rename `{name_str}` to `{expected}`."),
            )];
        }

        Vec::new()
    }
}

impl Cop for PredicatePrefix {
    fn name(&self) -> &'static str {
        "Naming/PredicatePrefix"
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["spec/**/*"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, DEF_NODE, SYMBOL_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Handle regular def nodes
        if let Some(def_node) = node.as_def_node() {
            let method_name = def_node.name().as_slice();
            let name_str = match std::str::from_utf8(method_name) {
                Ok(s) => s,
                Err(_) => return,
            };
            diagnostics.extend(self.check_method_name(
                source,
                name_str,
                def_node.name_loc().start_offset(),
                config,
            ));
        }

        // Handle MethodDefinitionMacros (e.g. define_method(:is_even))
        if let Some(call_node) = node.as_call_node() {
            let macros = config
                .get_string_array("MethodDefinitionMacros")
                .unwrap_or_else(|| vec!["define_method".into(), "define_singleton_method".into()]);

            let call_name = call_node.name().as_slice();
            let call_name_str = match std::str::from_utf8(call_name) {
                Ok(s) => s,
                Err(_) => return,
            };

            if !macros.iter().any(|m| m == call_name_str) {
                return;
            }

            // Only flag bare calls (no receiver), matching RuboCop's (send nil? :define_method ...)
            if call_node.receiver().is_some() {
                return;
            }

            // First argument should be a symbol literal with the method name
            let args = match call_node.arguments() {
                Some(a) => a,
                None => return,
            };
            let args_list: Vec<_> = args.arguments().iter().collect();
            if args_list.is_empty() {
                return;
            }
            if let Some(sym) = args_list[0].as_symbol_node() {
                let sym_bytes = sym.unescaped();
                let sym_str = match std::str::from_utf8(sym_bytes) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                diagnostics.extend(self.check_method_name(
                    source,
                    sym_str,
                    sym.location().start_offset(),
                    config,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(PredicatePrefix, "cops/naming/predicate_prefix");
}
