use crate::cop::shared::node_type::{ARRAY_NODE, CALL_NODE, HASH_NODE, KEYWORD_HASH_NODE};
use crate::cop::shared::util::as_method_chain;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Rails/WhereExists — enforces consistent `exists?` style.
///
/// ## Investigation (2026-03-10)
/// FP=74 root cause: "exists" style flagged ALL `where(...).exists?` regardless of argument types.
/// RuboCop's `convertable_args?` only flags when `where` args are hash, array, or multiple args.
/// Single string args (SQL fragments like `where("sql").exists?`), variables, and method calls
/// are NOT convertible and should not be flagged. Fixed by adding `convertible_args()` check.
///
/// "where" style had a secondary issue: flagged `exists?` with multiple args (e.g.,
/// `exists?('name = ?', 'john')`) but RuboCop's pattern only captures a single non-splat arg.
/// Fixed by requiring exactly one arg and skipping splat args.
///
/// ## Investigation (2026-03-14)
///
/// FP=43, FN=43 — equal counts per repo (location mismatch, not count mismatch).
/// Nitrocop reported offense at `node.location()` (start of the entire receiver chain),
/// but RuboCop reports at `node.receiver.loc.selector` (the `where` keyword).
/// Example: `Category.topic_create_allowed(guardian).where(id: @cat.id).exists?` —
/// nitrocop reported at line 267 (`Category`), RuboCop at line 269 (`where`).
///
/// Fix: Changed to report at `chain.inner_call.message_loc()` (the `where` keyword).
///
/// ## Investigation (2026-04-18)
///
/// Variant divergence only in `EnforcedStyle=where`: FP=28 / FN=28 were all
/// selector-range mismatches. nitrocop reported `exists?(...)` offenses at
/// `node.location()` (the start of the receiver chain, e.g. `Edition` or
/// `seller.sales`), while RuboCop reports at `node.loc.selector` (the
/// `exists?` keyword itself). Fixed by using `call.message_loc()` for the
/// `where` style offense position.
pub struct WhereExists;

impl Cop for WhereExists {
    fn name(&self) -> &'static str {
        "Rails/WhereExists"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[ARRAY_NODE, CALL_NODE, HASH_NODE, KEYWORD_HASH_NODE]
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
        let style = config.get_str("EnforcedStyle", "exists");

        let result = match style {
            "where" => self.check_where_style(source, node),
            _ => self.check_exists_style(source, node),
        };
        diagnostics.extend(result);
    }
}

impl WhereExists {
    /// "exists" style: flag `where(...).exists?`, suggest `exists?(...)`
    fn check_exists_style(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
    ) -> Vec<Diagnostic> {
        let chain = match as_method_chain(node) {
            Some(c) => c,
            None => return Vec::new(),
        };

        if chain.outer_method != b"exists?" {
            return Vec::new();
        }

        if chain.inner_method != b"where" {
            return Vec::new();
        }

        // The inner `where` call should have arguments
        let inner_args = match chain.inner_call.arguments() {
            Some(a) => a,
            None => return Vec::new(),
        };

        // Only flag when the where arguments are convertible to exists? args.
        // RuboCop checks: args.size > 1 || args[0].hash_type? || args[0].array_type?
        // Single string/variable/call args (SQL fragments) are NOT convertible.
        if !Self::convertible_args(inner_args) {
            return Vec::new();
        }

        // The outer `exists?` should NOT have arguments — if it does, the
        // developer is already passing conditions to exists? and this is a
        // different pattern (e.g., `where(a: 1).exists?(['sql', val])`)
        let outer_call = match node.as_call_node() {
            Some(c) => c,
            None => return Vec::new(),
        };
        if outer_call.arguments().is_some() {
            return Vec::new();
        }

        // Report at the `where` keyword location (matching RuboCop's correction_range which
        // spans from node.receiver.loc.selector — the `where` keyword — to node.loc.selector).
        // Using node.location() would report at the start of the entire receiver chain (e.g.,
        // line 267 for `Category.topic_create_allowed(...).where(...).exists?`), but RuboCop
        // reports at the `where` call (e.g., line 269).
        let where_loc = chain
            .inner_call
            .message_loc()
            .unwrap_or_else(|| chain.inner_call.location());
        let (line, column) = source.offset_to_line_col(where_loc.start_offset());
        vec![self.diagnostic(
            source,
            line,
            column,
            "Use `exists?(...)` instead of `where(...).exists?`.".to_string(),
        )]
    }

    /// "where" style: flag `exists?(...)` with arguments, suggest `where(...).exists?`
    fn check_where_style(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
    ) -> Vec<Diagnostic> {
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return Vec::new(),
        };

        if call.name().as_slice() != b"exists?" {
            return Vec::new();
        }

        // Must have arguments (exists? with args => should be where(...).exists?)
        let args = match call.arguments() {
            Some(a) => a,
            None => return Vec::new(),
        };

        // RuboCop's pattern: (call _ :exists? $!splat_type?)
        // This matches exists? with exactly one non-splat argument.
        // Then convertable_args? checks: hash_type? || array_type?
        let arg_list: Vec<_> = args.arguments().iter().collect();

        // Must have exactly one argument (multi-arg exists? is not flagged in "where" style)
        if arg_list.len() != 1 {
            return Vec::new();
        }

        let first = &arg_list[0];

        // Skip splat arguments: exists?(*conditions)
        if first.as_splat_node().is_some() {
            return Vec::new();
        }

        // Check that the arg is a hash, keyword hash, or array
        let is_convertible = first.as_hash_node().is_some()
            || first.as_keyword_hash_node().is_some()
            || first.as_array_node().is_some();

        if !is_convertible {
            return Vec::new();
        }

        // RuboCop's correction_range for `where` style starts at node.loc.selector,
        // i.e. the `exists?` keyword, not the beginning of the receiver chain.
        let loc = call.message_loc().unwrap_or(call.location());
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        vec![self.diagnostic(
            source,
            line,
            column,
            "Use `where(...).exists?` instead of `exists?(...)`.".to_string(),
        )]
    }

    /// Check if the arguments to `where(...)` are convertible to `exists?(...)`.
    /// RuboCop only converts hash, array, or multiple arguments — not single
    /// string args (SQL fragments), variables, or method calls.
    fn convertible_args(args: ruby_prism::ArgumentsNode<'_>) -> bool {
        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.is_empty() {
            return false;
        }
        // Multiple args: where('name = ?', 'john') — convertible
        if arg_list.len() > 1 {
            return true;
        }
        // Single arg: must be hash or array
        let first = &arg_list[0];
        first.as_hash_node().is_some()
            || first.as_keyword_hash_node().is_some()
            || first.as_array_node().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(WhereExists, "cops/rails/where_exists");

    fn where_config() -> crate::cop::CopConfig {
        use std::collections::HashMap;

        crate::cop::CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("where".to_string()),
            )]),
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn where_style_offense_fixture() {
        let fixture =
            include_bytes!("../../../tests/fixtures/cops/rails/where_exists/offense.where.rb");
        let fixture_str = std::str::from_utf8(fixture).expect("fixture must be valid UTF-8");
        let source = fixture_str
            .strip_prefix("# nitrocop-config: EnforcedStyle: where\n")
            .expect("fixture should start with where config directive");

        crate::testutil::assert_cop_offenses_full_with_config(
            &WhereExists,
            source.as_bytes(),
            where_config(),
        );
    }

    #[test]
    fn where_style_no_offense_fixture() {
        let fixture =
            include_bytes!("../../../tests/fixtures/cops/rails/where_exists/no_offense.where.rb");
        let fixture_str = std::str::from_utf8(fixture).expect("fixture must be valid UTF-8");
        let source = fixture_str
            .strip_prefix("# nitrocop-config: EnforcedStyle: where\n")
            .expect("fixture should start with where config directive");

        crate::testutil::assert_cop_no_offenses_full_with_config(
            &WhereExists,
            source.as_bytes(),
            where_config(),
        );
    }

    #[test]
    fn where_style_flags_exists_with_hash_arg() {
        use crate::testutil::run_cop_full_with_config;
        let source = b"User.exists?(name: 'john')\n";
        let diags = run_cop_full_with_config(&WhereExists, source, where_config());
        assert!(
            !diags.is_empty(),
            "where style should flag exists? with hash args"
        );
    }

    #[test]
    fn where_style_allows_where_exists() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        let source = b"User.where(name: 'john').exists?\n";
        assert_cop_no_offenses_full_with_config(&WhereExists, source, where_config());
    }

    #[test]
    fn where_style_skips_multi_arg_exists() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        // RuboCop does NOT flag exists? with multiple args in "where" style
        let source = b"User.exists?('name = ?', 'john')\n";
        assert_cop_no_offenses_full_with_config(&WhereExists, source, where_config());
    }

    #[test]
    fn where_style_skips_splat_arg() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        let source = b"User.exists?(*conditions)\n";
        assert_cop_no_offenses_full_with_config(&WhereExists, source, where_config());
    }
}
