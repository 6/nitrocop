use crate::cop::shared::node_type::{CALL_NODE, SYMBOL_NODE};
use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-07)
///
/// Corpus oracle reported FP=25, FN=9.
///
/// FP root cause: calls that pass a block argument (`&handler`) were treated as
/// hook blocks. RuboCop's matcher runs on `any_block` nodes only, so
/// `state.before(:each, &handler)`/`state.after(:each, &handler)` should be ignored.
///
/// Fix: require a literal Prism `BlockNode` (`do/end` or `{}`) before enforcing
/// hook-argument style.
///
/// Acceptance gate after fix (`check-cop --verbose --rerun`):
/// - Expected: 12,040
/// - Actual: 12,029
/// - Excess: 0
/// - Missing: 11 (remaining FN work deferred)
///
/// ## FN fix (2026-03-15)
///
/// 9 FNs from two root causes:
/// 1. `prepend_before`/`prepend_after`/`append_before`/`append_after` not in HOOK_METHODS (3 FN).
/// 2. Multi-argument hooks (`after(:each, type: :system)`) skipped by `arg_list.len() > 1` guard (6 FN).
///    RuboCop checks the first argument regardless of additional keyword args.
///
/// Fix: added all prepend/append hook variants to HOOK_METHODS, removed the multi-arg guard.
///
/// ## FP fix (2026-03-18)
///
/// FP=347, FN=0. The previous FN fix incorrectly removed the multi-arg guard entirely.
/// RuboCop's `scoped_hook` NodePattern is `(send _ #Hooks.all (sym ${:each :example}))`,
/// which requires **exactly one argument** — the scope symbol. Calls like
/// `before(:each, :special_tag)` or `after(:each, type: :system)` have additional
/// arguments so the pattern does NOT match and RuboCop does NOT flag them.
///
/// Fix: re-add `arg_list.len() > 1` guard for all style branches. Only flag when
/// the scope symbol is the sole argument.
///
/// ## Variant style FP fix (2026-04-06)
///
/// When `EnforcedStyle` is `each` or `example`, nitrocop incorrectly flagged scopes
/// like `:create`, `:build`, `:context`, `:suite` as "wrong style". For example,
/// `before(:create)` was flagged as "Use `before(:each)` instead of `before(:create)`"
/// when `EnforcedStyle: each`.
///
/// Root cause: the wrong-style check compared the scope symbol against the enforced
/// style without first verifying the scope is `:each` or `:example`. RuboCop's
/// `scoped_hook` pattern is `(any_block $(send _ #Hooks.all (sym ${:each :example})) ...)`,
/// which ONLY matches `:each` and `:example` — other scope symbols like `:create`,
/// `:build`, `:factory`, etc. are not in scope for this cop.
///
/// Fix: added early return in `each`/`example` style branches when the symbol is
/// neither `:each` nor `:example`. These scopes are simply ignored, matching RuboCop.
pub struct HookArgument;

/// Hook methods to check.
const HOOK_METHODS: &[&[u8]] = &[
    b"before",
    b"after",
    b"around",
    b"prepend_before",
    b"prepend_after",
    b"append_before",
    b"append_after",
];

/// Scope args that mean "suite" or "context" — not flagged.
const NON_EXAMPLE_SCOPES: &[&[u8]] = &[b"suite", b"context", b"all"];

impl Cop for HookArgument {
    fn name(&self) -> &'static str {
        "RSpec/HookArgument"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_DEFAULT_INCLUDE
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, SYMBOL_NODE]
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
        // Config: EnforcedStyle — "implicit" (default), "each", or "example"
        let enforced_style = config.get_str("EnforcedStyle", "implicit");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_name = call.name().as_slice();

        // Check both receiverless `before(...)` and `config.before(...)`
        let is_hook = HOOK_METHODS.contains(&method_name);

        if !is_hook {
            return;
        }

        // RuboCop matches `any_block` only; ignore block-pass args (`&handler`).
        if call.block().and_then(|b| b.as_block_node()).is_none() {
            return;
        }

        let args = call.arguments();
        let arg_list: Vec<_> = args
            .map(|a| a.arguments().iter().collect::<Vec<_>>())
            .unwrap_or_default();

        // For non-implicit styles with no arguments, flag the implicit usage
        if enforced_style == "each" || enforced_style == "example" {
            // No args = implicit — should have explicit arg
            if arg_list.is_empty() {
                let expected = enforced_style;
                let hook_name = std::str::from_utf8(method_name).unwrap_or("before");
                let loc = call.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!("Use `{hook_name}(:{expected})` instead of `{hook_name}`."),
                ));
                return;
            }

            // Multi-arg hooks are not flagged — RuboCop's NodePattern only matches
            // when the scope symbol is the sole argument.
            if arg_list.len() > 1 {
                return;
            }

            // Check for wrong style: e.g., enforced "each" but got :example
            if let Some(sym) = arg_list[0].as_symbol_node() {
                let val = sym.unescaped();

                // RuboCop's scoped_hook only matches :each and :example.
                // Other scope symbols (:create, :build, :suite, etc.) are not flagged.
                if val != b"each" && val != b"example" {
                    return;
                }

                if NON_EXAMPLE_SCOPES.contains(&val) {
                    return; // :suite/:context/:all are fine
                }
                let val_str = std::str::from_utf8(val).unwrap_or("");
                if val_str != enforced_style {
                    let hook_name = std::str::from_utf8(method_name).unwrap_or("before");
                    let loc = call.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        format!("Use `{hook_name}(:{enforced_style})` instead of `{hook_name}(:{val_str})`."),
                    ));
                }
            }

            return;
        }

        // Default: "implicit" style — flag :each and :example arguments
        // RuboCop's NodePattern matches only when the scope symbol is the sole argument.
        // Multi-arg hooks like `before(:each, :special_tag)` are not flagged.
        if arg_list.len() != 1 {
            return;
        }

        let first_arg = &arg_list[0];

        // Check for symbol argument
        if let Some(sym) = first_arg.as_symbol_node() {
            let val = sym.unescaped();

            // Ignore :suite, :context, :all — those are different scopes
            if NON_EXAMPLE_SCOPES.contains(&val) {
                return;
            }

            // Flag :each and :example — should be implicit
            if val == b"each" || val == b"example" {
                let scope_str = std::str::from_utf8(val).unwrap_or("each");
                let loc = call.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!("Omit the default `:{scope_str}` argument for RSpec hooks.",),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(HookArgument, "cops/rspec/hook_argument");

    #[test]
    fn each_style_flags_implicit() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("each".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"before do\n  setup\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&HookArgument, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("before(:each)"));
    }

    #[test]
    fn each_style_accepts_each() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("each".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"before(:each) do\n  setup\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&HookArgument, source, config);
        assert!(diags.is_empty());
    }

    #[test]
    fn each_style_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &HookArgument,
            include_bytes!("../../../tests/fixtures/cops/rspec/hook_argument/each_offense.rb"),
            {
                let mut options = std::collections::HashMap::new();
                options.insert(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("each".into()),
                );
                CopConfig {
                    options,
                    ..CopConfig::default()
                }
            },
        );
    }

    #[test]
    fn each_style_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HookArgument,
            include_bytes!("../../../tests/fixtures/cops/rspec/hook_argument/each_no_offense.rb"),
            {
                let mut options = std::collections::HashMap::new();
                options.insert(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("each".into()),
                );
                CopConfig {
                    options,
                    ..CopConfig::default()
                }
            },
        );
    }

    #[test]
    fn example_style_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &HookArgument,
            include_bytes!("../../../tests/fixtures/cops/rspec/hook_argument/example_offense.rb"),
            {
                let mut options = std::collections::HashMap::new();
                options.insert(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("example".into()),
                );
                CopConfig {
                    options,
                    ..CopConfig::default()
                }
            },
        );
    }

    #[test]
    fn example_style_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HookArgument,
            include_bytes!(
                "../../../tests/fixtures/cops/rspec/hook_argument/example_no_offense.rb"
            ),
            {
                let mut options = std::collections::HashMap::new();
                options.insert(
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("example".into()),
                );
                CopConfig {
                    options,
                    ..CopConfig::default()
                }
            },
        );
    }
}
