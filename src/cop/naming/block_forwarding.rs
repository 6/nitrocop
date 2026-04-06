use ruby_prism::Visit;

use crate::cop::shared::node_type::DEF_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-04)
///
/// Corpus oracle reported FP=3, FN=0.
///
/// FP=3: investigated examples from `samg__timetrap__edacc04` and
/// `rest-client__rest-client__2c72a2e`. Reducer validation indicated RuboCop
/// also reports offenses for the reduced patterns, suggesting artifact/location
/// noise rather than a stable semantic mismatch.
///
/// No code change applied in this batch. A future fix, if still needed after a
/// fresh rerun, should be based on exact offense-location/message diffs from a
/// regenerated corpus artifact.
///
/// ## Corpus investigation (2026-03-09)
///
/// Corpus oracle reported FP=1, FN=0.
///
/// FP=1 from rest-client (lib/restclient/request.rb:743): the `transmit` method
/// uses `& block` (with space between & and name) in the def signature, but
/// `&block` (no space) in body forwarding. RuboCop compares source text of the
/// param vs forwarding usage (`last_argument.source == block_pass_node.source`),
/// so `"& block" != "&block"` causes it to skip body offenses. nitrocop was
/// reporting the body offense because it matches by parsed name, not source text.
///
/// Fix: detect whitespace in the block param source (location length >
/// name length + 1) and skip body forwarding offenses when present. This
/// replicates RuboCop's source-comparison quirk.
///
/// ## Corpus investigation (2026-03-10)
///
/// Corpus oracle reported FP=0, FN=3. All 3 FNs from rest-client where
/// both param and body usage have matching whitespace (e.g., `& block` in
/// both positions). Attempted fix: per-usage source text comparison instead
/// of blanket `has_space_in_param` skip (commit 10a4cbe9, reverted 60638464).
///
/// Regression: the per-usage comparison caused FP=1138. Root cause not fully
/// determined but likely related to `visit_yield_node` counting `yield` as
/// forwarding (sets `has_forwarding=true`) while not adding to
/// `forwarding_locations`. When `has_forwarding || !has_any_reference` is true
/// and the param offense fires, the old blanket skip prevented body offenses
/// but the new per-usage loop emits them. Additionally, RuboCop's
/// `block_argument_name_matched?` has `return false if
/// block_pass_node.children.first&.sym_type?` which skips `&:method_name`
/// symbol block passes — our visitor may be matching those as forwarding.
///
/// Fix applied: when `has_space_in_param` is true, compare each forwarding
/// location's source text against the param's source text. If they match
/// (e.g., both are `& block`), emit the body offense. If they don't match
/// (e.g., param is `& block` but body is `&block`), skip it. This keeps
/// the default no-space path unchanged (all body offenses emitted) while
/// correctly handling the space-in-both case. The previous blanket skip
/// was too aggressive, suppressing body offenses even when sources matched.
/// Symbol block passes (`&:foo`) are already excluded by the visitor since
/// they don't match as `LocalVariableReadNode`.
///
/// ## Corpus investigation (2026-03-20)
///
/// Extended corpus oracle reported FP=2, FN=3.
///
/// FP=2 from `ahoward__sekrets__62af274` (test/lib/testing.rb:6,72): the outer
/// `def Testing(*args, &block)` method has `module_eval &block` forwarding inside
/// a `Class.new do...end` block, but a nested `def assert(*args, &block)` uses
/// `block` as a local variable (`block.call`, `if block`). RuboCop's
/// `use_block_argument_as_local_variable?` traverses into nested defs and finds
/// the inner lvar usage, skipping the outer method. nitrocop was stopping at def
/// boundaries.
///
/// Fix: removed the nested def barrier in the visitor (`visit_def_node` now
/// traverses into nested defs). This matches RuboCop's `each_descendant` behavior.
///
/// FN=2 from `RStankov__SearchObjectGraphQL__3257615` (graphql_spec.rb:22,49):
/// outer `def define_schema(&block)` has a nested `def initialize(*args, key: true,
/// **kwargs, &block)` with `super(*args, &block)`. RuboCop's `each_descendant`
/// finds the inner `&block` forwarding as a body offense for the outer method.
/// Same fix (nested def traversal).
///
/// FN=1 from `mongoid__mongoid-cached-json__b235c99` (config.rb:112):
/// `def transform(& block)` has a space in the param. The block is used as a
/// local variable (`items << block`), but RuboCop's `use_block_argument_as_local_variable?`
/// compares `last_argument.source[1..]` (= `" block"` with space) against lvar
/// names (`"block"`), so the comparison fails and the param offense fires.
///
/// Fix: when `has_space_in_param` is true, bypass the `only_forwarded` check and
/// fire the param offense unconditionally, matching RuboCop's source-text quirk.
///
/// ## Corpus investigation (2026-04-05)
///
/// Style variant `explicit` had 0 matches, 2,701 FN because `enforced_style != "anonymous"`
/// caused an early return — the explicit style was not implemented.
///
/// Fix: implemented the `explicit` style. When `EnforcedStyle: explicit`, flag
/// anonymous block params (`&` without a name) and anonymous block passes (`&`)
/// in the body, with message "Use explicit block forwarding." Autocorrect replaces
/// `&` with `&block` (or configured `BlockForwardingName`), skipping correction
/// when the name is already used by another parameter.
///
/// ## Corpus investigation (2026-04-06)
///
/// Explicit variant had 2,700 matches, FP=0, FN=1.
///
/// FN=1 from `crmne__ruby_llm__4135c03` (lib/ruby_llm/chat.rb:147): the pattern
/// `&wrap_streaming_block(&)` nests an anonymous `&` inside a non-anonymous block
/// pass expression. RuboCop's `each_descendant(:block_pass)` finds all block_pass
/// nodes regardless of nesting depth. `AnonBlockPassFinder` was not traversing
/// into block argument expressions, so the inner anonymous `&` was missed.
///
/// Fix: call `ruby_prism::visit_block_argument_node(self, node)` in the
/// `AnonBlockPassFinder` visitor to continue traversal into block argument
/// expressions, matching RuboCop's descendant search behavior.
pub struct BlockForwarding;

impl Cop for BlockForwarding {
    fn name(&self) -> &'static str {
        "Naming/BlockForwarding"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[DEF_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Anonymous block forwarding requires Ruby 3.1+
        // Default TargetRubyVersion is 3.4 (matching RuboCop's behavior when unset)
        let target_version = config
            .options
            .get("TargetRubyVersion")
            .and_then(|v| {
                v.as_f64()
                    .or_else(|| v.as_u64().map(|u| u as f64))
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(3.4);
        if target_version < 3.1 {
            return;
        }

        let enforced_style = config.get_str("EnforcedStyle", "anonymous");
        let _block_forwarding_name = config.get_str("BlockForwardingName", "block");

        let def_node = match node.as_def_node() {
            Some(d) => d,
            None => return,
        };

        let params = match def_node.parameters() {
            Some(p) => p,
            None => return,
        };

        // Check if there's a &block parameter
        let block_param = match params.block() {
            Some(b) => b,
            None => return,
        };

        if enforced_style == "explicit" {
            // Explicit style: flag anonymous block params (`&` without name)
            // If the param has a name, it's already explicit — no offense
            if block_param.name().is_some() {
                return;
            }

            let loc = block_param.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            let block_forwarding_name = config.get_str("BlockForwardingName", "block");

            // Check if the block forwarding name is already used as a parameter name
            let name_in_use = Self::block_forwarding_name_in_use(&def_node, block_forwarding_name);

            // Offense on the parameter
            let mut param_diag = self.diagnostic(
                source,
                line,
                column,
                "Use explicit block forwarding.".to_string(),
            );
            if let Some(ref mut corr) = corrections {
                if !name_in_use {
                    corr.push(crate::correction::Correction {
                        start: loc.start_offset(),
                        end: loc.end_offset(),
                        replacement: format!("&{}", block_forwarding_name),
                        cop_name: self.name(),
                        cop_index: 0,
                    });
                    param_diag.corrected = true;
                }
            }
            diagnostics.push(param_diag);

            // Find anonymous block passes in the body
            let mut finder = AnonBlockPassFinder {
                locations: Vec::new(),
            };
            if let Some(body) = def_node.body() {
                finder.visit(&body);
            }

            for (start, end) in &finder.locations {
                let (line, column) = source.offset_to_line_col(*start);
                let mut fwd_diag = self.diagnostic(
                    source,
                    line,
                    column,
                    "Use explicit block forwarding.".to_string(),
                );
                if let Some(ref mut corr) = corrections {
                    if !name_in_use {
                        corr.push(crate::correction::Correction {
                            start: *start,
                            end: *end,
                            replacement: format!("&{}", block_forwarding_name),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        fwd_diag.corrected = true;
                    }
                }
                diagnostics.push(fwd_diag);
            }

            return;
        }

        // Anonymous style (default): flag explicit block params that can be anonymous

        // If the block param has no name (already anonymous &), skip
        let param_name = match block_param.name() {
            Some(n) => n,
            None => return,
        };

        // Ruby has a syntax error when using anonymous block forwarding with keyword params:
        //   def foo(k:, &); end  => "no anonymous block parameter"
        // This applies to all Ruby versions that support anonymous block forwarding.
        if !params.keywords().is_empty() {
            return;
        }

        let param_name_bytes = param_name.as_slice();

        // Detect whitespace in the block param source (e.g., `& block` vs `&block`).
        // RuboCop's `use_block_argument_as_local_variable?` compares
        // `last_argument.source[1..]` against lvar names. When the param has a
        // space (source is `& block`), `source[1..]` is `" block"` which doesn't
        // match the lvar name `"block"`, so RuboCop fails to detect local variable
        // usage and fires the param offense anyway.
        let loc = block_param.location();
        let param_loc_len = loc.end_offset() - loc.start_offset();
        let has_space_in_param = param_loc_len > param_name_bytes.len() + 1;

        // Visit the body to check block param usage
        let mut checker = BlockUsageChecker {
            block_name: param_name_bytes,
            only_forwarded: true,
            has_forwarding: false,
            has_any_reference: false,
            used_in_nested_block: false,
            forwarding_locations: Vec::new(),
        };

        if let Some(body) = def_node.body() {
            checker.visit(&body);
        }
        // If body is None, the block param is unused — still an offense

        // If the block param is assigned (e.g., block ||= ...), it's not pure forwarding.
        // But when the param has a space (`& block`), RuboCop's lvar check fails
        // due to source text mismatch, so the param offense fires unconditionally.
        if !checker.only_forwarded && !has_space_in_param {
            return;
        }

        // Ruby 3.1-3.3: anonymous block forwarding inside nested blocks is a syntax error
        if target_version < 3.4 && checker.used_in_nested_block {
            return;
        }

        // Offense if:
        // - Block is forwarded (has_forwarding) and only forwarded (only_forwarded), OR
        // - Block is never referenced at all (unused param should be anonymous &), OR
        // - Param has space (RuboCop's lvar check broken, fires unconditionally)
        if checker.has_forwarding || !checker.has_any_reference || has_space_in_param {
            // Offense on the parameter: replace `&block` with `&`
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            let mut param_diag = self.diagnostic(
                source,
                line,
                column,
                "Use anonymous block forwarding.".to_string(),
            );
            if let Some(ref mut corr) = corrections {
                corr.push(crate::correction::Correction {
                    start: loc.start_offset(),
                    end: loc.end_offset(),
                    replacement: "&".to_string(),
                    cop_name: self.name(),
                    cop_index: 0,
                });
                param_diag.corrected = true;
            }
            diagnostics.push(param_diag);

            // Offense on each &block forwarding usage in the body.
            let param_source = if has_space_in_param {
                Some(&source.as_bytes()[loc.start_offset()..loc.end_offset()])
            } else {
                None
            };
            for (start, end) in &checker.forwarding_locations {
                if let Some(ps) = param_source {
                    let fwd_source = &source.as_bytes()[*start..*end];
                    if fwd_source != ps {
                        continue;
                    }
                }
                let (line, column) = source.offset_to_line_col(*start);
                let mut fwd_diag = self.diagnostic(
                    source,
                    line,
                    column,
                    "Use anonymous block forwarding.".to_string(),
                );
                if let Some(ref mut corr) = corrections {
                    corr.push(crate::correction::Correction {
                        start: *start,
                        end: *end,
                        replacement: "&".to_string(),
                        cop_name: self.name(),
                        cop_index: 0,
                    });
                    fwd_diag.corrected = true;
                }
                diagnostics.push(fwd_diag);
            }
        }
    }
}

impl BlockForwarding {
    /// Check if the block forwarding name (e.g., "block") is already used as a
    /// parameter name in this method definition.
    fn block_forwarding_name_in_use(def_node: &ruby_prism::DefNode<'_>, name: &str) -> bool {
        let params = match def_node.parameters() {
            Some(p) => p,
            None => return false,
        };
        let name_bytes = name.as_bytes();
        for req in params.requireds().iter() {
            if let Some(rp) = req.as_required_parameter_node() {
                if rp.name().as_slice() == name_bytes {
                    return true;
                }
            }
        }
        for opt in params.optionals().iter() {
            if let Some(op) = opt.as_optional_parameter_node() {
                if op.name().as_slice() == name_bytes {
                    return true;
                }
            }
        }
        if let Some(rest) = params.rest() {
            if let Some(rp) = rest.as_rest_parameter_node() {
                if let Some(n) = rp.name() {
                    if n.as_slice() == name_bytes {
                        return true;
                    }
                }
            }
        }
        for post in params.posts().iter() {
            if let Some(rp) = post.as_required_parameter_node() {
                if rp.name().as_slice() == name_bytes {
                    return true;
                }
            }
        }
        false
    }
}

/// Finds anonymous block pass nodes (`&` with no expression) in the body.
struct AnonBlockPassFinder {
    locations: Vec<(usize, usize)>,
}

impl<'pr> Visit<'pr> for AnonBlockPassFinder {
    fn visit_block_argument_node(&mut self, node: &ruby_prism::BlockArgumentNode<'pr>) {
        // Anonymous block pass: `&` with no expression child
        if node.expression().is_none() {
            let loc = node.location();
            self.locations.push((loc.start_offset(), loc.end_offset()));
        }
        // Continue traversing into the expression to find nested anonymous
        // block passes, e.g., `&wrap_streaming_block(&)` contains an inner `&`.
        // RuboCop's `each_descendant(:block_pass)` finds all descendants
        // regardless of nesting inside other block_pass expressions.
        ruby_prism::visit_block_argument_node(self, node);
    }

    fn visit_def_node(&mut self, _node: &ruby_prism::DefNode<'pr>) {
        // Don't traverse into nested defs — they have their own block context
    }
}

struct BlockUsageChecker<'a> {
    block_name: &'a [u8],
    only_forwarded: bool,
    has_forwarding: bool,
    has_any_reference: bool,
    used_in_nested_block: bool,
    /// (start_offset, end_offset) for each `&block` forwarding usage in the body
    forwarding_locations: Vec<(usize, usize)>,
}

impl<'pr> Visit<'pr> for BlockUsageChecker<'_> {
    fn visit_block_argument_node(&mut self, node: &ruby_prism::BlockArgumentNode<'pr>) {
        // &block in a call argument — this is forwarding
        if let Some(expr) = node.expression() {
            if let Some(local_var) = expr.as_local_variable_read_node() {
                if local_var.name().as_slice() == self.block_name {
                    self.has_forwarding = true;
                    self.has_any_reference = true;
                    // Collect the location of the &block argument for body-level offense
                    let loc = node.location();
                    self.forwarding_locations
                        .push((loc.start_offset(), loc.end_offset()));
                }
            } else {
                // Complex expression like &(block || fallback) — descend into children
                // to detect local variable reads of the block param, which make it
                // a non-forwarding use (can't use anonymous &).
                self.visit(&expr);
            }
        }
    }

    fn visit_yield_node(&mut self, node: &ruby_prism::YieldNode<'pr>) {
        // `yield` forwards the block — this counts as forwarding usage
        self.has_forwarding = true;
        // Continue visiting children (yield arguments)
        ruby_prism::visit_yield_node(self, node);
    }

    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        if node.name().as_slice() == self.block_name {
            // Block variable used in non-forwarding context (e.g., block.call, if block, block)
            // Note: reads inside block_argument_node are handled there, not here,
            // because visit_call_node visits block args before visiting regular args/receiver.
            self.only_forwarded = false;
            self.has_any_reference = true;
        }
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        if node.name().as_slice() == self.block_name {
            // Block param is reassigned — not pure forwarding
            self.only_forwarded = false;
            self.has_any_reference = true;
        }
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        if node.name().as_slice() == self.block_name {
            // block ||= ... — not pure forwarding
            self.only_forwarded = false;
            self.has_any_reference = true;
        }
        ruby_prism::visit_local_variable_or_write_node(self, node);
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        if node.name().as_slice() == self.block_name {
            self.only_forwarded = false;
            self.has_any_reference = true;
        }
        ruby_prism::visit_local_variable_and_write_node(self, node);
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        if node.name().as_slice() == self.block_name {
            self.only_forwarded = false;
            self.has_any_reference = true;
        }
        ruby_prism::visit_local_variable_operator_write_node(self, node);
    }

    fn visit_local_variable_target_node(
        &mut self,
        node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        if node.name().as_slice() == self.block_name {
            // Multi-assignment target (e.g., `a, block = ary`) — not pure forwarding
            self.only_forwarded = false;
            self.has_any_reference = true;
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        // Check the call's block argument first (so block arg reads are counted as forwarding)
        if let Some(block_arg) = node.block() {
            self.visit(&block_arg);
        }
        // Visit arguments
        if let Some(args) = node.arguments() {
            for arg in args.arguments().iter() {
                self.visit(&arg);
            }
        }
        // Visit receiver
        if let Some(recv) = node.receiver() {
            self.visit(&recv);
        }
    }

    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        // Track whether the block param is used inside a nested block (do..end / {})
        // On Ruby 3.1-3.3, anonymous block forwarding inside nested blocks is a syntax error.
        let saved_forwarding = self.has_forwarding;
        let saved_reference = self.has_any_reference;
        ruby_prism::visit_block_node(self, node);
        // If any forwarding or reference was added during the nested block visit,
        // mark used_in_nested_block
        if self.has_forwarding != saved_forwarding || self.has_any_reference != saved_reference {
            self.used_in_nested_block = true;
        }
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        // Same treatment as block_node for nested scope tracking
        let saved_forwarding = self.has_forwarding;
        let saved_reference = self.has_any_reference;
        ruby_prism::visit_lambda_node(self, node);
        if self.has_forwarding != saved_forwarding || self.has_any_reference != saved_reference {
            self.used_in_nested_block = true;
        }
    }

    // Traverse INTO nested defs. RuboCop's `each_descendant(:block_pass)`
    // and `use_block_argument_as_local_variable?` both cross def boundaries,
    // so we must do the same. Inner defs that define their own `&block` param
    // may shadow the outer one, but RuboCop doesn't distinguish by scope —
    // it matches by name only.
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        ruby_prism::visit_def_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(BlockForwarding, "cops/naming/block_forwarding");
    crate::cop_autocorrect_fixture_tests!(BlockForwarding, "cops/naming/block_forwarding");

    fn explicit_config() -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("explicit".to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn offense_explicit() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &BlockForwarding,
            include_bytes!(
                "../../../tests/fixtures/cops/naming/block_forwarding/offense.explicit.rb"
            ),
            explicit_config(),
        );
    }

    #[test]
    fn no_offense_explicit() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &BlockForwarding,
            include_bytes!(
                "../../../tests/fixtures/cops/naming/block_forwarding/no_offense.explicit.rb"
            ),
            explicit_config(),
        );
    }

    #[test]
    fn autocorrect_simple_forwarding() {
        let input = b"def foo(&block)\n  bar(&block)\nend\n";
        let (diags, corrections) = crate::testutil::run_cop_autocorrect(&BlockForwarding, input);
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().all(|d| d.corrected));
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"def foo(&)\n  bar(&)\nend\n");
    }

    #[test]
    fn autocorrect_unused_param() {
        let input = b"def foo(&block)\nend\n";
        let (diags, corrections) = crate::testutil::run_cop_autocorrect(&BlockForwarding, input);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].corrected);
        let cs = crate::correction::CorrectionSet::from_vec(corrections);
        let corrected = cs.apply(input);
        assert_eq!(corrected, b"def foo(&)\nend\n");
    }
}
