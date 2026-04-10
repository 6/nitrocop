use crate::cop::shared::util::keyword_arg_value;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

const SPECIFY_MSG: &str = "Specify an `:inverse_of` option.";
const NIL_MSG: &str =
    "You specified `inverse_of: nil`, you probably meant to use `inverse_of: false`.";

/// Rails/InverseOf -- checks `has_many`, `has_one`, and `belongs_to` associations
/// for missing `:inverse_of` option when Rails cannot automatically infer it.
///
/// ## Root causes of historical FNs (108 total, 0 FPs):
///
/// 1. **Only checked inside ClassNode bodies** -- the cop used `class_body_calls()`
///    which only found associations directly inside `class ... end` bodies. Most
///    FNs (90%+) were associations inside `included do ... end` blocks in
///    concern modules. Fixed by switching to `CALL_NODE` interested type, which
///    matches `has_many`/`has_one`/`belongs_to` calls anywhere in the file.
///
/// 2. **Missing `lambda { }` scope detection** -- the cop only checked for
///    `LambdaNode` (`-> {}`), but `lambda { order(:ordering) }` parses as a
///    `CallNode` with method name `lambda`. Fixed by also checking for
///    `CallNode` arguments with name `lambda`.
///
/// ## Round 2 fixes (4 FP, 1 FN):
///
/// 3. **`inverse_of: nil` not detected (FN)** -- `has_keyword_arg(inverse_of)`
///    returned true for `inverse_of: nil`, causing the cop to skip the offense.
///    RuboCop uses `!nil` NodePattern, treating `inverse_of: nil` as NOT having
///    the option set and emitting a different message. Fixed by checking the
///    value node for `NilNode`.
///
/// 4. **Dynamic options (`**options` kwsplat) not handled (FP)** -- when
///    association options contain `**hash`, the splat may dynamically provide
///    `inverse_of`. RuboCop's `dynamic_options?` check skips these unless
///    `inverse_of: nil` is explicitly present. Fixed by detecting
///    `AssocSplatNode` in keyword args.
///
/// 5. **`with_options` blocks not checked (FP)** -- RuboCop walks ancestor
///    `with_options` blocks to find `inverse_of` provided there. Switched from
///    `check_node` to `check_source` with a visitor to track enclosing
///    `with_options` context.
///
/// 6. **`foreign_key: nil` treated as requiring inverse_of (FP)** -- RuboCop's
///    `!nil` pattern means `foreign_key: nil` does not trigger the requirement.
///    Fixed by checking the value node is not nil.
///
/// ## Round 3 fixes (2 FP, 1 FN):
///
/// 7. **`with_options` providing `through:` or `polymorphic:` not tracked (FP)** --
///    `WithOptionsContext` only tracked `inverse_of`, `foreign_key`, and `conditions`.
///    When `through:` was in the enclosing `with_options` block (e.g., mastodon's
///    `has_many :voters` inside `with_options through: :votes`), the cop missed it
///    and flagged the association. Fixed by adding `has_through` and `has_polymorphic`
///    to `WithOptionsContext` and merging them in `check_association`.
///
/// 8. **Association calls with explicit receiver not matched (FN)** -- the visitor
///    required `node.receiver().is_none()`, excluding `base.has_many` calls in
///    concern `self.included(base)` patterns. RuboCop's `on_send` + NodePattern
///    matches any receiver. Fixed by matching on method name alone.
pub struct InverseOf;

impl Cop for InverseOf {
    fn name(&self) -> &'static str {
        "Rails/InverseOf"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/app/models/**/*.rb"]
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let ignore_scopes = config.get_bool("IgnoreScopes", false);

        let mut visitor = InverseOfVisitor {
            cop: self,
            source,
            ignore_scopes,
            // Stack of with_options contexts: each entry tracks whether inverse_of
            // is provided and whether options like foreign_key/conditions are set
            with_options_stack: Vec::new(),
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

/// Tracks what a `with_options` block contributes to enclosed associations.
struct WithOptionsContext {
    has_inverse_of: bool,
    has_foreign_key: bool,
    has_conditions: bool,
    has_through: bool,
    has_polymorphic: bool,
}

struct InverseOfVisitor<'a> {
    cop: &'a InverseOf,
    source: &'a SourceFile,
    ignore_scopes: bool,
    with_options_stack: Vec<WithOptionsContext>,
    diagnostics: Vec<Diagnostic>,
}

impl InverseOfVisitor<'_> {
    /// Check if any keyword hash argument contains an AssocSplatNode (**options).
    fn has_kwsplat(call: &ruby_prism::CallNode<'_>) -> bool {
        let Some(args) = call.arguments() else {
            return false;
        };
        for arg in args.arguments().iter() {
            if let Some(kw) = arg.as_keyword_hash_node() {
                for elem in kw.elements().iter() {
                    if elem.as_assoc_splat_node().is_some() {
                        return true;
                    }
                }
            }
            if let Some(hash) = arg.as_hash_node() {
                for elem in hash.elements().iter() {
                    if elem.as_assoc_splat_node().is_some() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check if a keyword arg exists and its value is not nil.
    /// Matches RuboCop's `(pair (sym :key) !nil)` pattern.
    fn has_keyword_arg_not_nil(call: &ruby_prism::CallNode<'_>, key: &[u8]) -> bool {
        match keyword_arg_value(call, key) {
            Some(val) => val.as_nil_node().is_none(),
            None => false,
        }
    }

    /// Check if inverse_of is explicitly set to nil.
    fn has_inverse_of_nil(call: &ruby_prism::CallNode<'_>) -> bool {
        match keyword_arg_value(call, b"inverse_of") {
            Some(val) => val.as_nil_node().is_some(),
            None => false,
        }
    }

    /// Extract with_options context from a call node's keyword arguments.
    fn extract_with_options_context(call: &ruby_prism::CallNode<'_>) -> WithOptionsContext {
        WithOptionsContext {
            has_inverse_of: Self::has_keyword_arg_not_nil(call, b"inverse_of"),
            has_foreign_key: Self::has_keyword_arg_not_nil(call, b"foreign_key"),
            has_conditions: Self::has_keyword_arg_not_nil(call, b"conditions"),
            has_through: Self::has_keyword_arg_not_nil(call, b"through"),
            has_polymorphic: Self::has_keyword_arg_not_nil(call, b"polymorphic"),
        }
    }

    fn check_association(&mut self, call: &ruby_prism::CallNode<'_>) {
        // Check if the call has a scope (lambda argument)
        let has_scope = call.arguments().is_some_and(|args| {
            args.arguments().iter().any(|a| {
                a.as_lambda_node().is_some()
                    || a.as_call_node()
                        .is_some_and(|c| c.name().as_slice() == b"lambda")
            })
        });

        // Gather options from the call itself AND from enclosing with_options blocks
        let has_through = Self::has_keyword_arg_not_nil(call, b"through")
            || self.with_options_stack.iter().any(|ctx| ctx.has_through);
        let has_polymorphic = Self::has_keyword_arg_not_nil(call, b"polymorphic")
            || self
                .with_options_stack
                .iter()
                .any(|ctx| ctx.has_polymorphic);

        // Skip associations with :through or :polymorphic
        if has_through || has_polymorphic {
            return;
        }

        let has_foreign_key = Self::has_keyword_arg_not_nil(call, b"foreign_key")
            || self
                .with_options_stack
                .iter()
                .any(|ctx| ctx.has_foreign_key);
        let has_conditions = Self::has_keyword_arg_not_nil(call, b"conditions")
            || self.with_options_stack.iter().any(|ctx| ctx.has_conditions);
        let needs_inverse = has_foreign_key || has_conditions || (has_scope && !self.ignore_scopes);

        if !needs_inverse {
            return;
        }

        // Check if inverse_of is provided (directly or via with_options)
        let has_inverse_of = Self::has_keyword_arg_not_nil(call, b"inverse_of")
            || self.with_options_stack.iter().any(|ctx| ctx.has_inverse_of);

        if has_inverse_of {
            return;
        }

        // Check for dynamic options (kwsplat) -- skip unless inverse_of: nil is present
        let has_dynamic = Self::has_kwsplat(call);
        let has_nil_inverse = Self::has_inverse_of_nil(call);

        if has_dynamic && !has_nil_inverse {
            return;
        }

        // Determine message
        let message = if has_nil_inverse {
            NIL_MSG.to_string()
        } else {
            SPECIFY_MSG.to_string()
        };

        let loc = call.message_loc().unwrap_or(call.location());
        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
        self.diagnostics
            .push(self.cop.diagnostic(self.source, line, column, message));
    }
}

impl<'pr> Visit<'pr> for InverseOfVisitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        // Check if this is a with_options block call
        if node.receiver().is_none()
            && node.name().as_slice() == b"with_options"
            && node.block().is_some()
        {
            let ctx = Self::extract_with_options_context(node);
            self.with_options_stack.push(ctx);

            // Visit children (the block body) with the context pushed
            if let Some(block) = node.block() {
                if let Some(block_node) = block.as_block_node() {
                    if let Some(body) = block_node.body() {
                        self.visit(&body);
                    }
                }
            }

            self.with_options_stack.pop();
            return;
        }

        // Check if this is an association call.
        // RuboCop matches any receiver (including none), e.g. `base.has_many` in
        // `self.included(base)` concern modules.
        let method = node.name();
        let is_assoc = method.as_slice() == b"has_many"
            || method.as_slice() == b"has_one"
            || method.as_slice() == b"belongs_to";

        if is_assoc {
            self.check_association(node);
        }

        // Continue visiting children for non-with_options calls
        // (e.g., included do ... end blocks)
        if let Some(args) = node.arguments() {
            self.visit(&args.as_node());
        }
        if let Some(block) = node.block() {
            self.visit(&block);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(InverseOf, "cops/rails/inverse_of");

    #[test]
    fn ignore_scopes_true_allows_scope_without_inverse_of() {
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([("IgnoreScopes".to_string(), serde_yml::Value::Bool(true))]),
            ..CopConfig::default()
        };
        let source =
            b"class Blog < ApplicationRecord\n  has_many :posts, -> { order(:name) }\nend\n";
        assert_cop_no_offenses_full_with_config(&InverseOf, source, config);
    }

    #[test]
    fn ignore_scopes_false_flags_scope_without_inverse_of() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;

        let config = CopConfig::default();
        let source =
            b"class Blog < ApplicationRecord\n  has_many :posts, -> { order(:name) }\nend\n";
        let diags = run_cop_full_with_config(&InverseOf, source, config);
        assert!(
            !diags.is_empty(),
            "IgnoreScopes:false should flag scope without inverse_of"
        );
    }
}
