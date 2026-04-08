use crate::cop::shared::node_type::{CALL_NODE, FORWARDING_SUPER_NODE, LAMBDA_NODE, SUPER_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Style/ItBlockParameter: checks for blocks where `it` block parameter can be used.
///
/// ## Investigation findings (2026-03-15)
///
/// The original implementation was fundamentally wrong — it only detected `|it|` named
/// block parameters, but RuboCop's cop handles implicit `it` blocks (Ruby 3.4+) and
/// numbered parameters (`_1`).
///
/// Root cause of FP=602: was flagging `|it|` named params which RuboCop never flags
/// in the default `allow_single_line` style.
///
/// Root cause of FN=1523: was not detecting multi-line implicit `it` blocks or `_1`
/// numbered parameters at all.
///
/// Rewritten to handle all 4 EnforcedStyle options:
/// - `allow_single_line` (default): flags multi-line `it` blocks + any `_1` usage
/// - `only_numbered_parameters`: flags only `_1` usage
/// - `always`: flags `_1` + single named params (should use `it` instead)
/// - `disallow`: flags all implicit `it` usage
///
/// Prism node mapping:
/// - `on_itblock` → `BlockNode` with `ItParametersNode` as parameters
/// - `on_numblock` → `BlockNode` with `NumberedParametersNode` (maximum == 1)
/// - `on_block` (always) → `BlockNode` with `BlockParametersNode` (single required param)
/// - `ItLocalVariableReadNode` = implicit `it` reference
/// - `LocalVariableReadNode` with name `_1` = numbered param reference
///
/// ## Investigation findings (2026-03-19)
///
/// FP=21, FN=43 remaining. Root causes:
///
/// 1. **Lambda blocks not handled (FN)** — `-> { _1 }` and `-> do it end` create
///    `LambdaNode` in Prism, not `CallNode`. RuboCop's `on_numblock`/`on_itblock`/`on_block`
///    fire for lambdas too. Fix: add `LAMBDA_NODE` to interested types and handle it.
///
/// 2. **Multi-line check is correct** — the `block_node.location()` check matches
///    RuboCop v1.84.2's `node.single_line?` which checks the block portion, NOT the
///    full expression including the receiver chain. Commit 8260fc1aa in RuboCop
///    explicitly reverted to this behavior ("allow_single_line targets the block,
///    not what is around it").
///
/// ## Investigation findings (2026-03-20)
///
/// FP=30, FN=1. Two root causes:
///
/// 1. **Bare `_1` as entire block body (FP=30)** — RuboCop's `find_block_variables`
///    uses `node.body.each_descendant(:lvar)` which does NOT include the body node
///    itself. When the block body IS the bare `_1` node (e.g., `{ _1 }`, `-> { _1 }`),
///    `each_descendant` returns nothing. nitrocop's Visit-based finder found these.
///    Fix: check if body has a single statement that is a bare `_1`/`it` read and skip it.
///
/// 2. **super with block (FN=1)** — `super { [_1.name, _1] }` creates a
///    `ForwardingSuperNode` in Prism, not a `CallNode`. The cop wasn't checking
///    ForwardingSuperNode or SuperNode. Fix: add both to interested_node_types.
///
/// ## Investigation findings (2026-04-06)
///
/// Variant `always` style FP (1,611 total): `block { |n| n }` where the body is a
/// bare reference to the named parameter. RuboCop's `on_block` with `always` style uses
/// `find_block_variables(node, 'n')` which calls `node.body.each_descendant(:lvar)`.
/// When the body IS the bare `n` node (not wrapped in a larger expression),
/// `each_descendant` returns nothing, so RuboCop does NOT flag it.
///
/// Fix: add `is_body_bare_named_param()` to skip when body is a bare named parameter
/// reference. This matches RuboCop's `each_descendant` semantics for both
/// `check_named_block` (CallNode blocks) and `check_lambda` (lambda blocks).
///
/// ## Investigation findings (2026-04-08)
///
/// Variant `always` style FN (31,792 total): `NamedParamFinder` stopped traversal
/// at nested block/lambda boundaries (`visit_block_node`/`visit_lambda_node` returned
/// early). RuboCop's `find_block_variables` uses `node.body.each_descendant(:lvar)`
/// which traverses ALL descendants including those inside nested blocks. Named
/// parameters can be captured by closures, so references in nested blocks must be found.
///
/// Fix: removed `visit_block_node` and `visit_lambda_node` overrides from
/// `NamedParamFinder`, restoring the default traversal behavior that descends into
/// all child nodes. `ItReferenceFinder` and `NumberedParamFinder` retain their
/// nested-block stops because `it`/`_1` are lexically scoped to their block.
pub struct ItBlockParameter;

impl Cop for ItBlockParameter {
    fn name(&self) -> &'static str {
        "Style/ItBlockParameter"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, LAMBDA_NODE, SUPER_NODE, FORWARDING_SUPER_NODE]
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
        // RuboCop: minimum_target_ruby_version 3.4
        let ruby_version = config
            .options
            .get("TargetRubyVersion")
            .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
            .unwrap_or(2.7);
        if ruby_version < 3.4 {
            return;
        }

        let style = config.get_str("EnforcedStyle", "allow_single_line");

        // Handle LambdaNode (-> { _1 }, -> { it }, -> { |x| x })
        if let Some(lambda) = node.as_lambda_node() {
            self.check_lambda(source, node, &lambda, style, diagnostics);
            return;
        }

        // Extract block from CallNode, SuperNode, or ForwardingSuperNode.
        // Each node type has its own block() return type, so we extract inline.
        if let Some(call) = node.as_call_node() {
            if let Some(block) = call.block() {
                if let Some(block_node) = block.as_block_node() {
                    self.check_block_params(source, node, &block_node, style, diagnostics);
                }
            }
            return;
        }
        if let Some(super_node) = node.as_super_node() {
            if let Some(block) = super_node.block() {
                if let Some(block_node) = block.as_block_node() {
                    self.check_block_params(source, node, &block_node, style, diagnostics);
                }
            }
            return;
        }
        if let Some(fwd_super) = node.as_forwarding_super_node() {
            if let Some(block_node) = fwd_super.block() {
                self.check_block_params(source, node, &block_node, style, diagnostics);
            }
        }
    }
}

impl ItBlockParameter {
    /// Dispatch to itblock/numblock/named-block checks based on block parameters.
    fn check_block_params(
        &self,
        source: &SourceFile,
        call_node: &ruby_prism::Node<'_>,
        block_node: &ruby_prism::BlockNode<'_>,
        style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let params = match block_node.parameters() {
            Some(p) => p,
            None => return,
        };

        if params.as_it_parameters_node().is_some() {
            self.check_itblock(source, call_node, block_node, style, diagnostics);
            return;
        }

        if let Some(numbered) = params.as_numbered_parameters_node() {
            self.check_numblock(source, block_node, &numbered, style, diagnostics);
            return;
        }

        if let Some(block_params) = params.as_block_parameters_node() {
            self.check_named_block(source, block_node, &block_params, style, diagnostics);
        }
    }

    /// Check implicit `it` blocks (ItParametersNode).
    /// - allow_single_line: flag multi-line blocks
    /// - disallow: flag all `it` references
    /// - only_numbered_parameters / always: no action on itblocks
    fn check_itblock(
        &self,
        source: &SourceFile,
        call_node: &ruby_prism::Node<'_>,
        block_node: &ruby_prism::BlockNode<'_>,
        style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match style {
            "allow_single_line" => {
                // Check if the block itself is multi-line (not the full expression).
                // This matches RuboCop v1.84.2's `node.single_line?` which checks the
                // block portion, allowing multi-line method chains with single-line blocks.
                let block_loc = block_node.location();
                let (start_line, _) = source.offset_to_line_col(block_loc.start_offset());
                let (end_line, _) =
                    source.offset_to_line_col(block_loc.end_offset().saturating_sub(1));
                if start_line == end_line {
                    return; // single-line, OK
                }
                // Offense on the call node (covers `block do`)
                let loc = call_node.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Avoid using `it` block parameter for multi-line blocks.".to_string(),
                ));
            }
            "disallow" => {
                // Flag each `it` reference in the block body
                if let Some(body) = block_node.body() {
                    // Match RuboCop's each_descendant: bare `it` as entire body not flagged
                    if Self::is_body_bare_it_param(&body) {
                        return;
                    }
                    let mut finder = ItReferenceFinder {
                        locations: Vec::new(),
                    };
                    finder.visit(&body);
                    for (start_offset, _end_offset) in finder.locations {
                        let (line, column) = source.offset_to_line_col(start_offset);
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Avoid using `it` block parameter.".to_string(),
                        ));
                    }
                }
            }
            // only_numbered_parameters, always: no offense for itblocks
            _ => {}
        }
    }

    /// Check lambda nodes (`-> { _1 }`, `-> { it }`, `-> { |x| x }`).
    /// In Prism, lambdas are `LambdaNode` (not `CallNode`), but RuboCop's
    /// `on_numblock`/`on_itblock`/`on_block` callbacks fire for lambdas too.
    fn check_lambda(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        lambda: &ruby_prism::LambdaNode<'_>,
        style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let params = match lambda.parameters() {
            Some(p) => p,
            None => return,
        };

        // Handle itblock (implicit `it` parameter in lambda)
        if params.as_it_parameters_node().is_some() {
            // For allow_single_line, check if the lambda is multi-line
            match style {
                "allow_single_line" => {
                    let loc = node.location();
                    let (start_line, _) = source.offset_to_line_col(loc.start_offset());
                    let (end_line, _) =
                        source.offset_to_line_col(loc.end_offset().saturating_sub(1));
                    if start_line == end_line {
                        return;
                    }
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Avoid using `it` block parameter for multi-line blocks.".to_string(),
                    ));
                }
                "disallow" => {
                    if let Some(body) = lambda.body() {
                        if Self::is_body_bare_it_param(&body) {
                            return;
                        }
                        let mut finder = ItReferenceFinder {
                            locations: Vec::new(),
                        };
                        finder.visit(&body);
                        for (start_offset, _end_offset) in finder.locations {
                            let (line, column) = source.offset_to_line_col(start_offset);
                            diagnostics.push(self.diagnostic(
                                source,
                                line,
                                column,
                                "Avoid using `it` block parameter.".to_string(),
                            ));
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        // Handle numblock (numbered parameters in lambda)
        if let Some(numbered) = params.as_numbered_parameters_node() {
            if style == "disallow" {
                return;
            }
            if numbered.maximum() != 1 {
                return;
            }
            if let Some(body) = lambda.body() {
                // Match RuboCop's each_descendant behavior: bare _1 as entire body is not flagged
                if Self::is_body_bare_numbered_param(&body) {
                    return;
                }
                let mut finder = NumberedParamFinder {
                    locations: Vec::new(),
                };
                finder.visit(&body);
                for (start_offset, _end_offset) in finder.locations {
                    let (line, column) = source.offset_to_line_col(start_offset);
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `it` block parameter.".to_string(),
                    ));
                }
            }
            return;
        }

        // Handle regular lambda with named params (only for `always` style)
        if style != "always" {
            return;
        }
        if let Some(block_params) = params.as_block_parameters_node() {
            let parameters = match block_params.parameters() {
                Some(p) => p,
                None => return,
            };
            let requireds = parameters.requireds();
            if requireds.len() != 1 {
                return;
            }
            if !parameters.optionals().is_empty()
                || parameters.rest().is_some()
                || !parameters.posts().is_empty()
                || !parameters.keywords().is_empty()
                || parameters.keyword_rest().is_some()
                || parameters.block().is_some()
            {
                return;
            }
            let param = match requireds.iter().next() {
                Some(p) => p,
                None => return,
            };
            let req_param = match param.as_required_parameter_node() {
                Some(rp) => rp,
                None => return,
            };
            let param_name = req_param.name();
            let body = match lambda.body() {
                Some(b) => b,
                None => return,
            };
            // RuboCop's find_block_variables uses each_descendant which doesn't include body
            // itself when it's a bare lvar. Match that behavior.
            if Self::is_body_bare_named_param(&body, param_name.as_slice()) {
                return;
            }
            let mut finder = NamedParamFinder {
                name: param_name.as_slice(),
                locations: Vec::new(),
            };
            finder.visit(&body);
            for (start_offset, _end_offset) in finder.locations {
                let (line, column) = source.offset_to_line_col(start_offset);
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Use `it` block parameter.".to_string(),
                ));
            }
        }
    }

    /// Check numbered parameter blocks (NumberedParametersNode).
    /// Flag `_1` usage when style is allow_single_line, only_numbered_parameters, or always.
    fn check_numblock(
        &self,
        source: &SourceFile,
        block_node: &ruby_prism::BlockNode<'_>,
        numbered: &ruby_prism::NumberedParametersNode<'_>,
        style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        // disallow style doesn't flag numbered params
        if style == "disallow" {
            return;
        }

        // Only flag when maximum == 1 (only _1 is used, no _2+)
        if numbered.maximum() != 1 {
            return;
        }

        if let Some(body) = block_node.body() {
            // Match RuboCop's `node.body.each_descendant(:lvar)` semantics:
            // When the body is a bare `_1` (single statement, no wrapping expression),
            // each_descendant on the body node itself finds no descendants (lvar is a leaf).
            if Self::is_body_bare_numbered_param(&body) {
                return;
            }
            let mut finder = NumberedParamFinder {
                locations: Vec::new(),
            };
            finder.visit(&body);
            for (start_offset, _end_offset) in finder.locations {
                let (line, column) = source.offset_to_line_col(start_offset);
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Use `it` block parameter.".to_string(),
                ));
            }
        }
    }

    /// Check if the body is a bare `_1` numbered parameter read (single statement, no wrapping).
    /// RuboCop's `node.body.each_descendant(:lvar)` doesn't find the body node itself when it's
    /// a leaf `lvar` node. This matches that behavior.
    fn is_body_bare_numbered_param(body: &ruby_prism::Node<'_>) -> bool {
        if let Some(stmts) = body.as_statements_node() {
            let body_nodes = stmts.body();
            if body_nodes.len() == 1 {
                if let Some(first) = body_nodes.iter().next() {
                    if let Some(lvar) = first.as_local_variable_read_node() {
                        return lvar.name().as_slice() == b"_1";
                    }
                }
            }
        }
        false
    }

    /// Check if the body is a bare implicit `it` read (single statement, no wrapping).
    /// Same as `is_body_bare_numbered_param` but for `ItLocalVariableReadNode`.
    fn is_body_bare_it_param(body: &ruby_prism::Node<'_>) -> bool {
        if let Some(stmts) = body.as_statements_node() {
            let body_nodes = stmts.body();
            if body_nodes.len() == 1 {
                if let Some(first) = body_nodes.iter().next() {
                    return first.as_it_local_variable_read_node().is_some();
                }
            }
        }
        false
    }

    /// Check if the body is a bare named parameter read (single statement, no wrapping).
    /// RuboCop's `node.body.each_descendant(:lvar)` doesn't find the body node itself
    /// when it's a leaf `lvar` node. This matches that behavior.
    fn is_body_bare_named_param(body: &ruby_prism::Node<'_>, param_name: &[u8]) -> bool {
        if let Some(stmts) = body.as_statements_node() {
            let body_nodes = stmts.body();
            if body_nodes.len() == 1 {
                if let Some(first) = body_nodes.iter().next() {
                    if let Some(lvar) = first.as_local_variable_read_node() {
                        return lvar.name().as_slice() == param_name;
                    }
                }
            }
        }
        false
    }

    /// Check regular blocks with named parameters (only for `always` style).
    /// Flags single-arg blocks where `it` could be used instead.
    fn check_named_block(
        &self,
        source: &SourceFile,
        block_node: &ruby_prism::BlockNode<'_>,
        block_params: &ruby_prism::BlockParametersNode<'_>,
        style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if style != "always" {
            return;
        }

        let parameters = match block_params.parameters() {
            Some(p) => p,
            None => return,
        };

        // Must have exactly one required parameter and no other params
        let requireds = parameters.requireds();
        if requireds.len() != 1 {
            return;
        }
        if !parameters.optionals().is_empty()
            || parameters.rest().is_some()
            || !parameters.posts().is_empty()
            || !parameters.keywords().is_empty()
            || parameters.keyword_rest().is_some()
            || parameters.block().is_some()
        {
            return;
        }

        let param = match requireds.iter().next() {
            Some(p) => p,
            None => return,
        };

        let req_param = match param.as_required_parameter_node() {
            Some(rp) => rp,
            None => return,
        };

        let param_name = req_param.name();

        // Need a body to find references
        let body = match block_node.body() {
            Some(b) => b,
            None => return,
        };

        // RuboCop's find_block_variables uses each_descendant which doesn't include body
        // itself when it's a bare lvar. Match that behavior.
        if Self::is_body_bare_named_param(&body, param_name.as_slice()) {
            return;
        }

        // Find all references to this parameter name in the body
        let mut finder = NamedParamFinder {
            name: param_name.as_slice(),
            locations: Vec::new(),
        };
        finder.visit(&body);

        for (start_offset, _end_offset) in finder.locations {
            let (line, column) = source.offset_to_line_col(start_offset);
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Use `it` block parameter.".to_string(),
            ));
        }
    }
}

/// Finds `ItLocalVariableReadNode` references (implicit `it`) in a block body.
struct ItReferenceFinder {
    locations: Vec<(usize, usize)>,
}

impl<'pr> Visit<'pr> for ItReferenceFinder {
    fn visit_it_local_variable_read_node(
        &mut self,
        node: &ruby_prism::ItLocalVariableReadNode<'pr>,
    ) {
        let loc = node.location();
        self.locations.push((loc.start_offset(), loc.end_offset()));
    }

    // Don't descend into nested blocks
    fn visit_block_node(&mut self, _node: &ruby_prism::BlockNode<'pr>) {}
    fn visit_lambda_node(&mut self, _node: &ruby_prism::LambdaNode<'pr>) {}
}

/// Finds `_1` references (LocalVariableReadNode with name "_1") in a block body.
struct NumberedParamFinder {
    locations: Vec<(usize, usize)>,
}

impl<'pr> Visit<'pr> for NumberedParamFinder {
    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        if node.name().as_slice() == b"_1" {
            let loc = node.location();
            self.locations.push((loc.start_offset(), loc.end_offset()));
        }
    }

    // Don't descend into nested blocks
    fn visit_block_node(&mut self, _node: &ruby_prism::BlockNode<'pr>) {}
    fn visit_lambda_node(&mut self, _node: &ruby_prism::LambdaNode<'pr>) {}
}

/// Finds references to a named local variable in a block body.
struct NamedParamFinder<'a> {
    name: &'a [u8],
    locations: Vec<(usize, usize)>,
}

impl<'pr, 'a> Visit<'pr> for NamedParamFinder<'a> {
    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        if node.name().as_slice() == self.name {
            let loc = node.location();
            self.locations.push((loc.start_offset(), loc.end_offset()));
        }
    }

    // Unlike ItReferenceFinder and NumberedParamFinder, NamedParamFinder DOES
    // descend into nested blocks. RuboCop's `find_block_variables` uses
    // `node.body.each_descendant(:lvar)` which traverses all descendants including
    // those inside nested blocks. Named parameters can be captured by closures,
    // so references in nested blocks must be found for the `always` style.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::CopConfig;

    fn ruby34_config() -> CopConfig {
        let mut config = CopConfig::default();
        config.options.insert(
            "TargetRubyVersion".to_string(),
            serde_yml::Value::Number(serde_yml::Number::from(3.4)),
        );
        config
    }

    fn always_style_config() -> CopConfig {
        let mut config = CopConfig::default();
        config.options.insert(
            "TargetRubyVersion".to_string(),
            serde_yml::Value::Number(3.4.into()),
        );
        config.options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("always".to_string()),
        );
        config
    }

    #[test]
    fn offense_with_ruby34() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ItBlockParameter,
            include_bytes!("../../../tests/fixtures/cops/style/it_block_parameter/offense.rb"),
            ruby34_config(),
        );
    }

    #[test]
    fn no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ItBlockParameter,
            include_bytes!("../../../tests/fixtures/cops/style/it_block_parameter/no_offense.rb"),
            ruby34_config(),
        );
    }

    #[test]
    fn no_offense_below_ruby34() {
        // Default Ruby version (2.7) — cop should be completely silent
        crate::testutil::assert_cop_no_offenses_full(
            &ItBlockParameter,
            include_bytes!("../../../tests/fixtures/cops/style/it_block_parameter/offense.rb"),
        );
    }

    #[test]
    fn offense_always_style() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ItBlockParameter,
            include_bytes!(
                "../../../tests/fixtures/cops/style/it_block_parameter/always_offense.rb"
            ),
            always_style_config(),
        );
    }

    #[test]
    fn lambda_always_style_direct_ref() {
        // Lambda with parenthesized params — the only valid lambda param syntax
        let source = b"-> (x) { x.to_s }\n";
        let diags = crate::testutil::run_cop_full_with_config(
            &ItBlockParameter,
            source,
            always_style_config(),
        );
        assert_eq!(diags.len(), 1, "Expected 1 offense: {:?}", diags);
    }

    #[test]
    fn lambda_always_style_nested_block() {
        // Lambda with named param used inside a nested block
        let source = b"-> (x) { bar { x.to_s } }\n";
        let diags = crate::testutil::run_cop_full_with_config(
            &ItBlockParameter,
            source,
            always_style_config(),
        );
        assert!(!diags.is_empty(), "Expected at least 1 offense but got 0");
    }

    #[test]
    fn no_offense_always_style_bare_named_param() {
        // Bare named param as entire body should NOT be flagged (FP fix).
        // RuboCop's find_block_variables uses each_descendant which doesn't include
        // the body node itself when it's a bare lvar.
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ItBlockParameter,
            include_bytes!(
                "../../../tests/fixtures/cops/style/it_block_parameter/no_offense_always.rb"
            ),
            always_style_config(),
        );
    }
}
