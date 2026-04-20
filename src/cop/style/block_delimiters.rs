use crate::cop::{CodeMap, Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;
use std::collections::HashSet;

/// Style/BlockDelimiters checks for uses of braces or do/end around single-line
/// or multi-line blocks.
///
/// ## Supported EnforcedStyle values
///
/// - `line_count_based` (default): single-line → braces, multi-line → do-end
/// - `always_braces`: always prefer braces
/// - `braces_for_chaining`: like line_count_based, but multi-line chained blocks use braces
/// - `semantic`: braces for functional blocks (return value used), do-end for procedural
///
/// ## Investigation findings (2026-04-05, variant styles)
///
/// Root cause of 506,090 FNs across three variant styles: the cop had an early return
/// `if enforced_style != "line_count_based" { return; }` that skipped ALL processing for
/// non-default styles. Implemented:
///
/// - `always_braces`: flags any `do...end` block
/// - `braces_for_chaining`: detects chained blocks (call is receiver of another call)
///   and allows braces on chained multi-line blocks while requiring do-end on non-chained
/// - `semantic`: detects return-value usage via parent context (assignment, chaining,
///   argument position, last-in-scope) to distinguish functional vs procedural blocks
///
/// ## Fix (2026-04-16): restore semantic wrapper contexts RuboCop sees in Parser AST
///
/// The remaining semantic mismatches came from Prism wrappers around the block's
/// call expression:
///
/// - `return map { ... }` was missed because `ReturnNode` did not forward
///   "last child of scope" to its final argument
/// - `assert_equal(*args.map { ... })` was missed because `SplatNode` did not
///   mark its wrapped expression as `rv_of_scope`
/// - `(items.map do ... end).join(",")` was missed because parenthesized
///   receivers were not forwarded into `rv_used` / chained detection
/// - rescue/ensure bodies with exactly one statement were treated as ordinary
///   visits, so their tail expression never got `rv_of_scope`
///
/// RuboCop treats all of those wrappers as part of the semantic-parent check for
/// `return_value_used?` / `return_value_of_scope?`, so nitrocop now propagates
/// through those wrappers before classifying the block as functional/procedural.
///
/// ## Fix (2026-04-16): match RuboCop's ignored lambda bodies and `yield` wrapper
///
/// Two remaining variant mismatches came from Prism-only wrapper shapes:
///
/// - `yield records.filter_map { ... }` is functional in RuboCop because the
///   block expression is the last child of a `yield`, so braces are allowed
/// - `register_placeholder :path, -> do { raw_value: foo.tap do ... end } end`
///   and interpolated-string lambda bodies stay ignored in RuboCop, but our
///   lambda-body walker stopped before `HashNode` / interpolation wrappers
/// - `queue << -> do logger.info { ... } end` must NOT use that ignore path:
///   RuboCop treats a lambda passed as the sole operator argument like a block
///   argument and still checks nested blocks inside it
///
/// ## Fix (2026-04-17): match RuboCop's semantic tail-equality and syntax bail-out
///
/// Two remaining variant mismatches were RuboCop quirks rather than new style rules:
///
/// - `parent.children.last == node` in RuboCop compares AST nodes structurally,
///   not by identity, so an earlier block statement that is structurally equal
///   to the true tail expression (for example a repeated `assert_queries(1) { ... }`
///   or `reverse.each { ... }` before a final `reverse.each do ... end`) also
///   counts as `rv_of_scope`
/// - Prism stores `&Proc.new {}` on `call.block()` as a `BlockArgumentNode`,
///   not in `call.arguments()`, so semantic style must still mark the wrapped
///   `Proc.new` call as `rv_used`
/// - Prism folds `foo[bar] ||= baz` into `IndexOrWriteNode`, so the block call
///   inside the receiver must be marked `rv_used` even though there is no
///   intermediate `CallNode` for `[]`
/// - Under `EnforcedStyle: always_braces`, RuboCop's Translation::Parser also
///   bails out on some non-UTF-8 files with high `\xHH` escapes (for example
///   `# encoding:windows-1252` with `\xdf` regex escapes), so nitrocop now
///   skips this cop on those parser-incompatible files too
///
/// ## Fix (2026-04-20): narrow always_braces bail-outs and semantic wrappers
///
/// The remaining variant divergence came from four Prism-specific edge cases:
///
/// - `always_braces` was skipping the cop on *any* Prism error, which hid real
///   offenses in builder templates where Prism reports semantic-only `yield`
///   errors but RuboCop still checks block delimiters
/// - the non-UTF-8 bailout was too broad: RuboCop still checks ordinary
///   ISO-8859-1/Windows-1252 files with raw high bytes, and only bails on the
///   parser-incompatible high `\xHH` escape cases
/// - semantic style did not forward return-value-of-scope through `return`,
///   `break`, `next`, or optional-parameter defaults, so blocks in those
///   wrapper contexts were flagged as procedural
/// - Prism omits `message_loc` for shorthand `receiver.(...)`, so the old call
///   key collided with the receiver's start offset and falsely marked the block
///   call as return-value-used; additionally, the scope-tail equality fallback
///   stripped all whitespace from source text, so `"a b"` and `"ab"` looked
///   structurally equal and earlier brace blocks incorrectly inherited
///   `rv_of_scope`
pub struct BlockDelimiters;

impl Cop for BlockDelimiters {
    fn name(&self) -> &'static str {
        "Style/BlockDelimiters"
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
        let enforced_style = config.get_str("EnforcedStyle", "line_count_based");
        if enforced_style == "always_braces"
            && has_non_utf8_encoding_with_parser_incompatible_content(source.as_bytes())
        {
            return;
        }
        let procedural_methods = config
            .get_string_array("ProceduralMethods")
            .unwrap_or_else(|| vec!["tap".to_string()]);
        let functional_methods = config
            .get_string_array("FunctionalMethods")
            .unwrap_or_else(|| vec!["let".to_string()]);
        let allowed_methods = config.get_string_array("AllowedMethods");
        let allowed_patterns = config.get_string_array("AllowedPatterns");
        let allow_braces_on_procedural = config.get_bool("AllowBracesOnProceduralOneLiners", false);
        let braces_required_methods = config.get_string_array("BracesRequiredMethods");

        let allowed = allowed_methods
            .unwrap_or_else(|| vec!["lambda".to_string(), "proc".to_string(), "it".to_string()]);
        let patterns = allowed_patterns.unwrap_or_default();
        let braces_required = braces_required_methods.unwrap_or_default();

        let mut visitor = BlockDelimitersVisitor {
            source,
            cop: self,
            diagnostics: Vec::new(),
            ignored_blocks: HashSet::new(),
            suppressed_ranges: Vec::new(),
            allowed_methods: allowed,
            allowed_patterns: patterns,
            braces_required_methods: braces_required,
            enforced_style,
            chained_blocks: HashSet::new(),
            rv_used_calls: HashSet::new(),
            rv_of_scope_calls: HashSet::new(),
            procedural_methods,
            functional_methods,
            allow_braces_on_procedural_one_liners: allow_braces_on_procedural,
            is_program_body: true,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct BlockDelimitersVisitor<'a> {
    source: &'a SourceFile,
    cop: &'a BlockDelimiters,
    diagnostics: Vec<Diagnostic>,
    ignored_blocks: HashSet<usize>,
    /// Byte ranges of blocks that suppress nested block checks.
    /// Includes: (1) blocks in non-parenthesized arg positions (binding change),
    /// (2) blocks that already received an offense (RuboCop `ignore_node` behavior).
    suppressed_ranges: Vec<(usize, usize)>,
    allowed_methods: Vec<String>,
    allowed_patterns: Vec<String>,
    braces_required_methods: Vec<String>,
    enforced_style: &'a str,
    /// Block opening offsets that are chained (call is receiver of another call).
    chained_blocks: HashSet<usize>,
    /// Call node start offsets whose return value is used (for semantic style).
    rv_used_calls: HashSet<usize>,
    /// Call node start offsets in scope-return position (for semantic style).
    rv_of_scope_calls: HashSet<usize>,
    procedural_methods: Vec<String>,
    functional_methods: Vec<String>,
    allow_braces_on_procedural_one_liners: bool,
    /// True until the first StatementsNode is visited (program body).
    /// In Parser AST, single-statement programs have no `begin` wrapper,
    /// so rv_of_scope is false for the single top-level expression.
    /// Multi-statement programs have a `begin` wrapper where the last
    /// child gets rv_of_scope. We replicate this by only marking the
    /// last child of the program body when there are multiple statements.
    is_program_body: bool,
}

impl<'a> BlockDelimitersVisitor<'a> {
    /// Check if a block's byte range is contained within any suppressed range.
    fn is_suppressed(&self, start: usize, end: usize) -> bool {
        self.suppressed_ranges
            .iter()
            .any(|&(s, e)| s <= start && end <= e)
    }

    /// Add a byte range to the suppressed set.
    ///
    /// Callers should pass the **call node's** range (not just the block node's)
    /// so that chained blocks are properly suppressed. In Prism, chained calls
    /// like `a.select { }.reject { }` have the outermost CallNode covering the
    /// entire chain, while BlockNode ranges only cover their own `{...}`.
    fn suppress_range(&mut self, start: usize, end: usize) {
        self.suppressed_ranges.push((start, end));
    }

    fn check_block(
        &mut self,
        block_node: &ruby_prism::BlockNode<'_>,
        method_name: &[u8],
        call_start: usize,
    ) -> bool {
        let method_str = std::str::from_utf8(method_name).unwrap_or("");

        // Skip AllowedMethods (default: lambda, proc, it)
        if self.allowed_methods.iter().any(|m| m == method_str) {
            return false;
        }

        // Skip AllowedPatterns
        for pattern in &self.allowed_patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if re.is_match(method_str) {
                    return false;
                }
            }
        }

        let opening_loc = block_node.opening_loc();
        let closing_loc = block_node.closing_loc();
        let opening = opening_loc.as_slice();
        let is_braces = opening == b"{";

        let (open_line, _) = self.source.offset_to_line_col(opening_loc.start_offset());
        let (close_line, _) = self.source.offset_to_line_col(closing_loc.start_offset());
        let is_single_line = open_line == close_line;

        // BracesRequiredMethods: must use braces (takes precedence over style)
        if self.braces_required_methods.iter().any(|m| m == method_str) {
            if !is_braces {
                let (line, column) = self.source.offset_to_line_col(opening_loc.start_offset());
                self.diagnostics.push(self.cop.diagnostic(
                    self.source,
                    line,
                    column,
                    format!(
                        "Brace delimiters `{{...}}` required for '{}' method.",
                        method_str
                    ),
                ));
                return true;
            }
            return false;
        }

        // require_do_end: single-line do-end blocks with rescue/ensure clauses
        // cannot be converted to braces (syntax error). Skip these.
        if is_single_line && !is_braces && block_has_rescue_or_ensure(block_node) {
            return false;
        }

        match self.enforced_style {
            "line_count_based" => {
                self.check_line_count_based(block_node, is_single_line, is_braces)
            }
            "always_braces" => self.check_always_braces(block_node, is_braces),
            "braces_for_chaining" => {
                let is_chained = self
                    .chained_blocks
                    .contains(&block_node.opening_loc().start_offset());
                self.check_braces_for_chaining(block_node, is_single_line, is_braces, is_chained)
            }
            "semantic" => {
                let is_chained = self
                    .chained_blocks
                    .contains(&block_node.opening_loc().start_offset());
                let rv_used = self.rv_used_calls.contains(&call_start) || is_chained;
                let rv_of_scope = self.rv_of_scope_calls.contains(&call_start);
                self.check_semantic(
                    block_node,
                    method_name,
                    is_single_line,
                    !is_single_line,
                    is_braces,
                    rv_used,
                    rv_of_scope,
                )
            }
            _ => false,
        }
    }

    fn check_line_count_based(
        &mut self,
        block_node: &ruby_prism::BlockNode<'_>,
        is_single_line: bool,
        is_braces: bool,
    ) -> bool {
        // line_count_based: multiline ^ braces → proper
        if is_single_line && !is_braces {
            self.emit_offense(
                block_node,
                "Prefer `{...}` over `do...end` for single-line blocks.",
            );
            true
        } else if !is_single_line && is_braces {
            self.emit_offense(
                block_node,
                "Prefer `do...end` over `{...}` for multi-line blocks.",
            );
            true
        } else {
            false
        }
    }

    fn check_always_braces(
        &mut self,
        block_node: &ruby_prism::BlockNode<'_>,
        is_braces: bool,
    ) -> bool {
        if !is_braces {
            self.emit_offense(block_node, "Prefer `{...}` over `do...end` for blocks.");
            true
        } else {
            false
        }
    }

    fn check_braces_for_chaining(
        &mut self,
        block_node: &ruby_prism::BlockNode<'_>,
        is_single_line: bool,
        is_braces: bool,
        is_chained: bool,
    ) -> bool {
        if is_single_line {
            // Single-line: prefer braces
            if !is_braces {
                self.emit_offense(
                    block_node,
                    "Prefer `{...}` over `do...end` for single-line blocks.",
                );
                return true;
            }
        } else {
            // Multi-line
            if is_chained {
                // Chained: prefer braces
                if !is_braces {
                    self.emit_offense(
                        block_node,
                        "Prefer `{...}` over `do...end` for multi-line chained blocks.",
                    );
                    return true;
                }
            } else {
                // Not chained: prefer do-end
                if is_braces {
                    self.emit_offense(
                        block_node,
                        "Prefer `do...end` for multi-line blocks without chaining.",
                    );
                    return true;
                }
            }
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn check_semantic(
        &mut self,
        block_node: &ruby_prism::BlockNode<'_>,
        method_name: &[u8],
        is_single_line: bool,
        _is_multiline: bool,
        is_braces: bool,
        rv_used: bool,
        rv_of_scope: bool,
    ) -> bool {
        let method_str = std::str::from_utf8(method_name).unwrap_or("");
        let is_functional_method = self.functional_methods.iter().any(|m| m == method_str);
        let is_procedural_method = self.procedural_methods.iter().any(|m| m == method_str);
        let functional_block = rv_used || rv_of_scope;

        if is_braces {
            // Proper if: functional_method, or functional_block, or (allow_one_liners && single-line)
            let proper = is_functional_method
                || functional_block
                || (self.allow_braces_on_procedural_one_liners && is_single_line);
            if !proper {
                self.emit_offense(
                    block_node,
                    "Prefer `do...end` over `{...}` for procedural blocks.",
                );
                return true;
            }
        } else {
            // do-end: proper if procedural_method or return value not used
            let proper = is_procedural_method || !rv_used;
            if !proper {
                self.emit_offense(
                    block_node,
                    "Prefer `{...}` over `do...end` for functional blocks.",
                );
                return true;
            }
        }
        false
    }

    fn emit_offense(&mut self, block_node: &ruby_prism::BlockNode<'_>, message: &str) {
        let opening_loc = block_node.opening_loc();
        let (line, column) = self.source.offset_to_line_col(opening_loc.start_offset());
        self.diagnostics.push(
            self.cop
                .diagnostic(self.source, line, column, message.to_string()),
        );
    }
}

impl<'a> Visit<'_> for BlockDelimitersVisitor<'a> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'_>) {
        // For non-parenthesized calls with arguments, mark argument blocks
        // as ignored. Changing delimiters on these blocks would change binding
        // semantics (braces bind tighter than do..end).
        // `[]` method calls (e.g., `Hash[x]`) use square brackets, not parens.
        // In Prism, `opening_loc()` returns `Some` for `[`, but RuboCop treats
        // `[]` calls as non-parenthesized for block-binding purposes.
        let method_name = node.name().as_slice();
        let is_parenthesized = node.opening_loc().is_some() && method_name != b"[]";
        let is_assignment = method_name.ends_with(b"=")
            && method_name != b"=="
            && method_name != b"!="
            && method_name != b"<="
            && method_name != b">="
            && method_name != b"===";

        // Skip operator methods with a single block-bearing argument.
        let is_single_arg_operator = is_operator_method(method_name)
            && node.arguments().is_some_and(|args| {
                args.arguments().len() == 1
                    && args
                        .arguments()
                        .iter()
                        .next()
                        .is_some_and(|arg| single_argument_operator_block_arg(&arg))
            });

        if !is_parenthesized && !is_assignment && !is_single_arg_operator {
            if let Some(args) = node.arguments() {
                for arg in args.arguments().iter() {
                    collect_ignored_blocks(&arg, &mut self.ignored_blocks);
                }
            }
        }

        // Pre-mark context for chaining and return-value detection.
        if let Some(receiver) = node.receiver() {
            mark_rv_used_on_call(&receiver, &mut self.rv_used_calls);
            mark_chained_receiver_block(&receiver, &mut self.chained_blocks);
        }

        // Arguments to this call have their return values used.
        if let Some(args) = node.arguments() {
            for arg in args.arguments().iter() {
                mark_rv_used_on_call(&arg, &mut self.rv_used_calls);
            }
        }
        if let Some(block_arg) = node.block() {
            mark_rv_used_on_call(&block_arg, &mut self.rv_used_calls);
        }

        // Phase 2: Check this call's block (if any)
        if let Some(block) = node.block() {
            if let Some(block_node) = block.as_block_node() {
                let offset = block_node.opening_loc().start_offset();
                let block_end = block_node.closing_loc().end_offset();

                let call_range_start = node.location().start_offset();
                let call_end = node.location().end_offset();
                let call_key = call_node_key(node);

                if self.ignored_blocks.contains(&offset) {
                    self.suppress_range(call_range_start, call_end);
                } else if !self.is_suppressed(offset, block_end) {
                    let flagged = self.check_block(&block_node, method_name, call_key);
                    if flagged {
                        self.suppress_range(call_range_start, call_end);
                    }
                }
            }
        }

        // Recurse into children
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_super_node(&mut self, node: &ruby_prism::SuperNode<'_>) {
        if let Some(args) = node.arguments() {
            if let Some(last) = args.arguments().last() {
                mark_rv_of_scope_on_node(&last, &mut self.rv_of_scope_calls);
            }
        }

        if let Some(block) = node.block() {
            if let Some(block_node) = block.as_block_node() {
                let offset = block_node.opening_loc().start_offset();
                let block_end = block_node.closing_loc().end_offset();
                let call_range_start = node.location().start_offset();
                let call_end = node.location().end_offset();
                let call_key = node.keyword_loc().start_offset();

                if !self.is_suppressed(offset, block_end) {
                    let flagged = self.check_block(&block_node, b"super", call_key);
                    if flagged {
                        self.suppress_range(call_range_start, call_end);
                    }
                }
            }
        }
        ruby_prism::visit_super_node(self, node);
    }

    fn visit_forwarding_super_node(&mut self, node: &ruby_prism::ForwardingSuperNode<'_>) {
        if let Some(block_node) = node.block() {
            let offset = block_node.opening_loc().start_offset();
            let block_end = block_node.closing_loc().end_offset();
            let call_range_start = node.location().start_offset();
            let call_end = node.location().end_offset();
            let call_key = node.location().start_offset();

            if !self.is_suppressed(offset, block_end) {
                let flagged = self.check_block(&block_node, b"super", call_key);
                if flagged {
                    self.suppress_range(call_range_start, call_end);
                }
            }
        }
        ruby_prism::visit_forwarding_super_node(self, node);
    }

    fn visit_yield_node(&mut self, node: &ruby_prism::YieldNode<'_>) {
        // In Parser AST, a block passed to `yield` is a direct child of the
        // yield node, so `parent.children.last == node` makes it functional.
        if let Some(args) = node.arguments() {
            if let Some(last) = args.arguments().last() {
                mark_rv_of_scope_on_node(&last, &mut self.rv_of_scope_calls);
            }
        }
        ruby_prism::visit_yield_node(self, node);
    }

    // --- Context tracking for semantic & braces_for_chaining styles ---

    fn visit_statements_node(&mut self, node: &ruby_prism::StatementsNode<'_>) {
        // Mark the last statement's call as rv_of_scope (return value of scope).
        // RuboCop uses AST `==` here, so earlier siblings that are structurally
        // equal to the actual last child also count as rv_of_scope.
        let body: Vec<_> = node.body().iter().collect();
        if self.is_program_body {
            // Program body: only mark if multiple statements (matches Parser's
            // begin wrapper — single-statement files have no begin, so block.parent
            // is nil and rv_of_scope is false)
            self.is_program_body = false;
            if body.len() > 1 {
                mark_rv_of_scope_on_statement_tail_matches(
                    &body,
                    self.source,
                    &mut self.rv_of_scope_calls,
                );
            }
        } else {
            // Non-program body (def, block, class, etc.): always mark last child
            mark_rv_of_scope_on_statement_tail_matches(
                &body,
                self.source,
                &mut self.rv_of_scope_calls,
            );
        }
        ruby_prism::visit_statements_node(self, node);
    }

    fn visit_assoc_node(&mut self, node: &ruby_prism::AssocNode<'_>) {
        // In RuboCop/Parser AST, a block used as the value in `key: value` has
        // parent `:pair`, so `parent.children.last == node` makes it rv_of_scope.
        mark_rv_of_scope_on_node(&node.value(), &mut self.rv_of_scope_calls);
        ruby_prism::visit_assoc_node(self, node);
    }

    fn visit_assoc_splat_node(&mut self, node: &ruby_prism::AssocSplatNode<'_>) {
        if let Some(value) = node.value() {
            mark_rv_of_scope_on_node(&value, &mut self.rv_of_scope_calls);
        }
        ruby_prism::visit_assoc_splat_node(self, node);
    }

    fn visit_splat_node(&mut self, node: &ruby_prism::SplatNode<'_>) {
        if let Some(expression) = node.expression() {
            mark_rv_of_scope_on_node(&expression, &mut self.rv_of_scope_calls);
        }
        ruby_prism::visit_splat_node(self, node);
    }

    fn visit_return_node(&mut self, node: &ruby_prism::ReturnNode<'_>) {
        if let Some(args) = node.arguments() {
            if let Some(last) = args.arguments().last() {
                mark_rv_of_scope_on_node(&last, &mut self.rv_of_scope_calls);
            }
        }
        ruby_prism::visit_return_node(self, node);
    }

    fn visit_break_node(&mut self, node: &ruby_prism::BreakNode<'_>) {
        if let Some(args) = node.arguments() {
            if let Some(last) = args.arguments().last() {
                mark_rv_of_scope_on_node(&last, &mut self.rv_of_scope_calls);
            }
        }
        ruby_prism::visit_break_node(self, node);
    }

    fn visit_next_node(&mut self, node: &ruby_prism::NextNode<'_>) {
        if let Some(args) = node.arguments() {
            if let Some(last) = args.arguments().last() {
                mark_rv_of_scope_on_node(&last, &mut self.rv_of_scope_calls);
            }
        }
        ruby_prism::visit_next_node(self, node);
    }

    fn visit_optional_parameter_node(&mut self, node: &ruby_prism::OptionalParameterNode<'_>) {
        mark_rv_of_scope_on_node(&node.value(), &mut self.rv_of_scope_calls);
        ruby_prism::visit_optional_parameter_node(self, node);
    }

    fn visit_optional_keyword_parameter_node(
        &mut self,
        node: &ruby_prism::OptionalKeywordParameterNode<'_>,
    ) {
        mark_rv_of_scope_on_node(&node.value(), &mut self.rv_of_scope_calls);
        ruby_prism::visit_optional_keyword_parameter_node(self, node);
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'_>) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_instance_variable_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_instance_variable_write_node(self, node);
    }

    fn visit_class_variable_write_node(&mut self, node: &ruby_prism::ClassVariableWriteNode<'_>) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_class_variable_write_node(self, node);
    }

    fn visit_global_variable_write_node(&mut self, node: &ruby_prism::GlobalVariableWriteNode<'_>) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_global_variable_write_node(self, node);
    }

    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'_>) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_constant_write_node(self, node);
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_local_variable_operator_write_node(self, node);
    }

    fn visit_instance_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOperatorWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_instance_variable_operator_write_node(self, node);
    }

    fn visit_class_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOperatorWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_class_variable_operator_write_node(self, node);
    }

    fn visit_global_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOperatorWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_global_variable_operator_write_node(self, node);
    }

    fn visit_constant_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantOperatorWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_constant_operator_write_node(self, node);
    }

    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'_>) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_constant_path_write_node(self, node);
    }

    fn visit_constant_path_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOperatorWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_constant_path_operator_write_node(self, node);
    }

    fn visit_call_operator_write_node(&mut self, node: &ruby_prism::CallOperatorWriteNode<'_>) {
        if let Some(receiver) = node.receiver() {
            mark_rv_used_on_call(&receiver, &mut self.rv_used_calls);
        }
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_call_operator_write_node(self, node);
    }

    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'_>) {
        if let Some(receiver) = node.receiver() {
            mark_rv_used_on_call(&receiver, &mut self.rv_used_calls);
        }
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_call_and_write_node(self, node);
    }

    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'_>) {
        if let Some(receiver) = node.receiver() {
            mark_rv_used_on_call(&receiver, &mut self.rv_used_calls);
        }
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_call_or_write_node(self, node);
    }

    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'_>) {
        if let Some(receiver) = node.receiver() {
            mark_rv_used_on_call(&receiver, &mut self.rv_used_calls);
        }
        if let Some(args) = node.arguments() {
            for arg in args.arguments().iter() {
                mark_rv_used_on_call(&arg, &mut self.rv_used_calls);
            }
        }
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_index_operator_write_node(self, node);
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'_>) {
        if let Some(receiver) = node.receiver() {
            mark_rv_used_on_call(&receiver, &mut self.rv_used_calls);
        }
        if let Some(args) = node.arguments() {
            for arg in args.arguments().iter() {
                mark_rv_used_on_call(&arg, &mut self.rv_used_calls);
            }
        }
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_index_and_write_node(self, node);
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'_>) {
        if let Some(receiver) = node.receiver() {
            mark_rv_used_on_call(&receiver, &mut self.rv_used_calls);
        }
        if let Some(args) = node.arguments() {
            for arg in args.arguments().iter() {
                mark_rv_used_on_call(&arg, &mut self.rv_used_calls);
            }
        }
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_index_or_write_node(self, node);
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_local_variable_and_write_node(self, node);
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_local_variable_or_write_node(self, node);
    }

    fn visit_instance_variable_and_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableAndWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_instance_variable_and_write_node(self, node);
    }

    fn visit_instance_variable_or_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOrWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_instance_variable_or_write_node(self, node);
    }

    fn visit_class_variable_and_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableAndWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_class_variable_and_write_node(self, node);
    }

    fn visit_class_variable_or_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOrWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_class_variable_or_write_node(self, node);
    }

    fn visit_global_variable_and_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableAndWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_global_variable_and_write_node(self, node);
    }

    fn visit_global_variable_or_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOrWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_global_variable_or_write_node(self, node);
    }

    fn visit_constant_and_write_node(&mut self, node: &ruby_prism::ConstantAndWriteNode<'_>) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_constant_and_write_node(self, node);
    }

    fn visit_constant_or_write_node(&mut self, node: &ruby_prism::ConstantOrWriteNode<'_>) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_constant_or_write_node(self, node);
    }

    fn visit_constant_path_and_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathAndWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_constant_path_and_write_node(self, node);
    }

    fn visit_constant_path_or_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOrWriteNode<'_>,
    ) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_constant_path_or_write_node(self, node);
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'_>) {
        mark_rv_used_on_call(&node.value(), &mut self.rv_used_calls);
        ruby_prism::visit_multi_write_node(self, node);
    }

    // Conditional and logical contexts mark contents as rv_of_scope
    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'_>) {
        // RuboCop treats condition predicates as return-value-of-scope, not
        // return-value-used. This keeps `if find do ... end` procedural.
        mark_rv_of_scope_on_node(&node.predicate(), &mut self.rv_of_scope_calls);
        ruby_prism::visit_if_node(self, node);
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'_>) {
        mark_rv_of_scope_on_node(&node.predicate(), &mut self.rv_of_scope_calls);
        ruby_prism::visit_unless_node(self, node);
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'_>) {
        mark_rv_of_scope_on_node(&node.predicate(), &mut self.rv_of_scope_calls);
        ruby_prism::visit_while_node(self, node);
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'_>) {
        mark_rv_of_scope_on_node(&node.predicate(), &mut self.rv_of_scope_calls);
        ruby_prism::visit_until_node(self, node);
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'_>) {
        // Both sides of `and`/`or` are in rv_of_scope position
        mark_rv_of_scope_on_node(&node.left(), &mut self.rv_of_scope_calls);
        mark_rv_of_scope_on_node(&node.right(), &mut self.rv_of_scope_calls);
        ruby_prism::visit_and_node(self, node);
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'_>) {
        mark_rv_of_scope_on_node(&node.left(), &mut self.rv_of_scope_calls);
        mark_rv_of_scope_on_node(&node.right(), &mut self.rv_of_scope_calls);
        ruby_prism::visit_or_node(self, node);
    }

    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'_>) {
        // Elements of array literals have their return values used
        for element in node.elements().iter() {
            mark_rv_of_scope_on_node(&element, &mut self.rv_of_scope_calls);
        }
        ruby_prism::visit_array_node(self, node);
    }

    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'_>) {
        if let Some(predicate) = node.predicate() {
            mark_rv_of_scope_on_node(&predicate, &mut self.rv_of_scope_calls);
        }
        ruby_prism::visit_case_node(self, node);
    }

    fn visit_case_match_node(&mut self, node: &ruby_prism::CaseMatchNode<'_>) {
        if let Some(predicate) = node.predicate() {
            mark_rv_of_scope_on_node(&predicate, &mut self.rv_of_scope_calls);
        }
        ruby_prism::visit_case_match_node(self, node);
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'_>) {
        let has_rescue_or_ensure = node.rescue_clause().is_some() || node.ensure_clause().is_some();

        if has_rescue_or_ensure {
            if let Some(stmts) = node.statements() {
                if stmts.body().len() == 1 {
                    // Parser keeps a single begin-with-rescue body statement
                    // directly under `:rescue`/`:ensure`, so it is not
                    // implicitly rv_of_scope.
                    for child in stmts.body().iter() {
                        self.visit(&child);
                    }
                } else {
                    self.visit_statements_node(&stmts);
                }
            }
            if let Some(rescue_clause) = node.rescue_clause() {
                self.visit_rescue_node(&rescue_clause);
            }
            if let Some(else_clause) = node.else_clause() {
                self.visit_else_node(&else_clause);
            }
            if let Some(ensure_clause) = node.ensure_clause() {
                self.visit_ensure_node(&ensure_clause);
            }
        } else {
            ruby_prism::visit_begin_node(self, node);
        }
    }

    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'_>) {
        for exception in node.exceptions().iter() {
            self.visit(&exception);
        }
        if let Some(reference) = node.reference() {
            self.visit(&reference);
        }

        if let Some(stmts) = node.statements() {
            if stmts.body().len() == 1 {
                for child in stmts.body().iter() {
                    mark_rv_of_scope_on_node(&child, &mut self.rv_of_scope_calls);
                    self.visit(&child);
                }
            } else {
                self.visit_statements_node(&stmts);
            }
        }

        if let Some(subsequent) = node.subsequent() {
            self.visit_rescue_node(&subsequent);
        }
    }

    fn visit_ensure_node(&mut self, node: &ruby_prism::EnsureNode<'_>) {
        if let Some(stmts) = node.statements() {
            if stmts.body().len() == 1 {
                for child in stmts.body().iter() {
                    mark_rv_of_scope_on_node(&child, &mut self.rv_of_scope_calls);
                    self.visit(&child);
                }
            } else {
                self.visit_statements_node(&stmts);
            }
        }
    }

    fn visit_range_node(&mut self, node: &ruby_prism::RangeNode<'_>) {
        if let Some(left) = node.left() {
            mark_rv_of_scope_on_node(&left, &mut self.rv_of_scope_calls);
        }
        if let Some(right) = node.right() {
            mark_rv_of_scope_on_node(&right, &mut self.rv_of_scope_calls);
        }
        ruby_prism::visit_range_node(self, node);
    }
}

/// Mark a node as having its return value used (for semantic style).
/// Only marks if the node is a CallNode, SuperNode, or ForwardingSuperNode.
/// Unique key for a CallNode — uses the method name offset (message_loc)
/// instead of the call's full location. In Prism, chained calls like
/// `a.b.c` all share the same start_offset (the location includes the
/// receiver), so using start_offset would mark ALL calls in a chain as
/// rv_used when only the receiver is. The method name is unique per call.
fn call_node_key(call: &ruby_prism::CallNode<'_>) -> usize {
    call.message_loc()
        .or_else(|| call.call_operator_loc())
        .or_else(|| call.opening_loc())
        .map_or_else(|| call.location().end_offset(), |loc| loc.start_offset())
}

fn mark_rv_used_on_call(node: &ruby_prism::Node<'_>, rv_used: &mut HashSet<usize>) {
    if let Some(call) = node.as_call_node() {
        rv_used.insert(call_node_key(&call));
    } else if let Some(super_node) = node.as_super_node() {
        rv_used.insert(super_node.keyword_loc().start_offset());
    } else if let Some(fwd_super) = node.as_forwarding_super_node() {
        rv_used.insert(fwd_super.location().start_offset());
    } else if let Some(block_arg) = node.as_block_argument_node() {
        if let Some(expression) = block_arg.expression() {
            mark_rv_used_on_call(&expression, rv_used);
        }
    } else if let Some(parens) = node.as_parentheses_node() {
        // Propagate through parentheses: `(map do ... end)` → rv_used
        if let Some(body) = parens.body() {
            if let Some(stmts) = body.as_statements_node() {
                for stmt in stmts.body().iter() {
                    mark_rv_used_on_call(&stmt, rv_used);
                }
            }
        }
    }
}

fn mark_chained_receiver_block(node: &ruby_prism::Node<'_>, chained_blocks: &mut HashSet<usize>) {
    if let Some(call) = node.as_call_node() {
        if let Some(block_node) = call.block().and_then(|block| block.as_block_node()) {
            chained_blocks.insert(block_node.opening_loc().start_offset());
        }
    }
    // Note: do NOT recurse through ParenthesesNode here. RuboCop's
    // braces_for_chaining style doesn't consider `(foo.map do...end).join`
    // as a chained block — the parens break the chain. Only the semantic
    // style needs parenthesized receiver detection, and that is handled
    // by rv_used/rv_of_scope propagation, not the chained_blocks set.
}

/// Mark a node as being in return-value-of-scope position (for semantic style).
fn mark_rv_of_scope_on_node(node: &ruby_prism::Node<'_>, rv_of_scope: &mut HashSet<usize>) {
    if let Some(call) = node.as_call_node() {
        rv_of_scope.insert(call_node_key(&call));
    } else if let Some(super_node) = node.as_super_node() {
        rv_of_scope.insert(super_node.keyword_loc().start_offset());
    } else if let Some(fwd_super) = node.as_forwarding_super_node() {
        rv_of_scope.insert(fwd_super.location().start_offset());
    } else if let Some(ret) = node.as_return_node() {
        if let Some(args) = ret.arguments() {
            if let Some(last) = args.arguments().last() {
                mark_rv_of_scope_on_node(&last, rv_of_scope);
            }
        }
    } else if let Some(paren) = node.as_parentheses_node() {
        if let Some(body) = paren.body() {
            if let Some(stmts) = body.as_statements_node() {
                if let Some(last) = stmts.body().last() {
                    mark_rv_of_scope_on_node(&last, rv_of_scope);
                }
            } else {
                mark_rv_of_scope_on_node(&body, rv_of_scope);
            }
        }
    } else if let Some(splat) = node.as_splat_node() {
        if let Some(expression) = splat.expression() {
            mark_rv_of_scope_on_node(&expression, rv_of_scope);
        }
    } else if let Some(hash) = node.as_hash_node() {
        // Recursively mark calls in hash values
        for element in hash.elements().iter() {
            if let Some(assoc) = element.as_assoc_node() {
                mark_rv_of_scope_on_node(&assoc.value(), rv_of_scope);
            } else if let Some(splat) = element.as_assoc_splat_node() {
                if let Some(value) = splat.value() {
                    mark_rv_of_scope_on_node(&value, rv_of_scope);
                }
            }
        }
    } else if let Some(arr) = node.as_array_node() {
        // Recursively mark calls in array elements
        for element in arr.elements().iter() {
            mark_rv_of_scope_on_node(&element, rv_of_scope);
        }
    } else if let Some(kwh) = node.as_keyword_hash_node() {
        // Recursively mark calls in keyword hash values
        for element in kwh.elements().iter() {
            if let Some(assoc) = element.as_assoc_node() {
                mark_rv_of_scope_on_node(&assoc.value(), rv_of_scope);
            } else if let Some(splat) = element.as_assoc_splat_node() {
                if let Some(value) = splat.value() {
                    mark_rv_of_scope_on_node(&value, rv_of_scope);
                }
            }
        }
    }
}

fn mark_rv_of_scope_on_statement_tail_matches(
    statements: &[ruby_prism::Node<'_>],
    source: &SourceFile,
    rv_of_scope: &mut HashSet<usize>,
) {
    let Some(last) = statements.last() else {
        return;
    };

    for statement in statements {
        if statement_matches_scope_tail(statement, last, source) {
            mark_rv_of_scope_on_node(statement, rv_of_scope);
        }
    }
}

fn statement_matches_scope_tail(
    candidate: &ruby_prism::Node<'_>,
    last: &ruby_prism::Node<'_>,
    source: &SourceFile,
) -> bool {
    let candidate_loc = candidate.location();
    let last_loc = last.location();
    if candidate_loc.start_offset() == last_loc.start_offset()
        && candidate_loc.end_offset() == last_loc.end_offset()
    {
        return true;
    }

    match (candidate.as_call_node(), last.as_call_node()) {
        (Some(candidate_call), Some(last_call)) => {
            block_calls_match_scope_tail(&candidate_call, &last_call, source)
        }
        _ => false,
    }
}

fn block_calls_match_scope_tail(
    candidate: &ruby_prism::CallNode<'_>,
    last: &ruby_prism::CallNode<'_>,
    source: &SourceFile,
) -> bool {
    let (Some(candidate_block), Some(last_block)) = (
        candidate.block().and_then(|block| block.as_block_node()),
        last.block().and_then(|block| block.as_block_node()),
    ) else {
        return false;
    };

    if candidate.name().as_slice() != last.name().as_slice() {
        return false;
    }

    source_slice_matches(
        source,
        candidate.location().start_offset(),
        candidate_block.opening_loc().start_offset(),
        last.location().start_offset(),
        last_block.opening_loc().start_offset(),
    ) && optional_node_source_matches(
        candidate_block.parameters(),
        last_block.parameters(),
        source,
    ) && optional_node_source_matches(candidate_block.body(), last_block.body(), source)
}

fn optional_node_source_matches(
    candidate: Option<ruby_prism::Node<'_>>,
    last: Option<ruby_prism::Node<'_>>,
    source: &SourceFile,
) -> bool {
    match (candidate, last) {
        (None, None) => true,
        (Some(candidate_node), Some(last_node)) => {
            let candidate_loc = candidate_node.location();
            let last_loc = last_node.location();
            source_slice_matches(
                source,
                candidate_loc.start_offset(),
                candidate_loc.end_offset(),
                last_loc.start_offset(),
                last_loc.end_offset(),
            )
        }
        _ => false,
    }
}

fn source_slice_matches(
    source: &SourceFile,
    first_start: usize,
    first_end: usize,
    second_start: usize,
    second_end: usize,
) -> bool {
    source.as_bytes().get(first_start..first_end) == source.as_bytes().get(second_start..second_end)
}

fn has_non_utf8_encoding_with_parser_incompatible_content(bytes: &[u8]) -> bool {
    let mut start = 0;
    for _ in 0..3 {
        let end = bytes[start..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map(|pos| start + pos)
            .unwrap_or(bytes.len());
        let line = &bytes[start..end];
        let trimmed: Vec<u8> = line.iter().copied().filter(|byte| *byte != b'\r').collect();

        if trimmed.starts_with(b"#") {
            let lower: Vec<u8> = trimmed
                .iter()
                .map(|byte| byte.to_ascii_lowercase())
                .collect();
            if let Some(pos) = find_subsequence(&lower, b"encoding")
                .or_else(|| find_subsequence(&lower, b"coding"))
            {
                let after = &lower[pos..];
                let value_start = after
                    .iter()
                    .position(|&byte| byte == b':' || byte == b'=')
                    .map(|pos| pos + 1)
                    .unwrap_or(after.len());
                let value = &after[value_start..];
                let value_trimmed: Vec<u8> = value
                    .iter()
                    .copied()
                    .skip_while(|byte| *byte == b' ')
                    .collect();
                let enc_end = value_trimmed
                    .iter()
                    .position(|byte| {
                        !byte.is_ascii_alphanumeric() && *byte != b'-' && *byte != b'_'
                    })
                    .unwrap_or(value_trimmed.len());
                let enc_name = &value_trimmed[..enc_end];

                if enc_name == b"utf"
                    || enc_name == b"utf8"
                    || enc_name.starts_with(b"utf-8")
                    || enc_name.starts_with(b"utf_8")
                    || enc_name == b"binary"
                    || enc_name.starts_with(b"ascii-8bit")
                    || enc_name.starts_with(b"ascii_8bit")
                    || enc_name == b"us-ascii"
                    || enc_name == b"ascii"
                {
                    return false;
                }

                if !enc_name.is_empty() {
                    return has_high_hex_escapes(bytes);
                }
            }
        }

        start = end + 1;
        if start >= bytes.len() {
            break;
        }
    }
    false
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn has_high_hex_escapes(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }

    for window in bytes.windows(4) {
        if window[0] == b'\\' && window[1] == b'x' {
            let high = window[2];
            let low = window[3];
            let high_is_8_plus = matches!(high, b'8'..=b'9' | b'a'..=b'f' | b'A'..=b'F');
            if high_is_8_plus && low.is_ascii_hexdigit() {
                return true;
            }
        }
    }

    false
}

/// Check if a block corresponds to Parser's `:block` type (not `:itblock` or `:numblock`).
/// Returns false for blocks with `it` parameters (`:itblock` in Parser) or numbered
/// parameters like `_1` (`:numblock` in Parser). Blocks with explicit parameters
/// (`|x, y|`) or no parameters at all are both `:block` type.
fn is_explicit_block(block: ruby_prism::BlockNode<'_>) -> bool {
    match block.parameters() {
        Some(p) => {
            // ItParametersNode → :itblock, NumberedParametersNode → :numblock
            p.as_it_parameters_node().is_none() && p.as_numbered_parameters_node().is_none()
        }
        // No parameters → :block type
        None => true,
    }
}

fn single_argument_operator_block_arg(node: &ruby_prism::Node<'_>) -> bool {
    if node.as_lambda_node().is_some() {
        return true;
    }

    node.as_call_node()
        .and_then(|call| call.block())
        .and_then(|block| block.as_block_node())
        .is_some_and(is_explicit_block)
}

/// Check if a method name is a Ruby operator method.
/// Matches RuboCop's `OPERATOR_METHODS` from `MethodIdentifierPredicates`.
fn is_operator_method(name: &[u8]) -> bool {
    matches!(
        name,
        b"|" | b"^"
            | b"&"
            | b"<=>"
            | b"=="
            | b"==="
            | b"=~"
            | b">"
            | b">="
            | b"<"
            | b"<="
            | b"<<"
            | b">>"
            | b"+"
            | b"-"
            | b"*"
            | b"/"
            | b"%"
            | b"**"
            | b"~"
            | b"+@"
            | b"-@"
            | b"!@"
            | b"~@"
            | b"[]"
            | b"[]="
            | b"!"
            | b"!="
            | b"!~"
            | b"`"
    )
}

/// Check if a block's body contains rescue or ensure clauses.
/// In Prism, this manifests as a BeginNode body with rescue_clause or ensure_clause.
fn block_has_rescue_or_ensure(block_node: &ruby_prism::BlockNode<'_>) -> bool {
    if let Some(body) = block_node.body() {
        if let Some(begin_node) = body.as_begin_node() {
            return begin_node.rescue_clause().is_some() || begin_node.ensure_clause().is_some();
        }
    }
    false
}

/// Recursively collect blocks inside argument expressions of non-parenthesized
/// method calls. These blocks must be ignored because changing `{...}` to
/// `do...end` (or vice versa) would change block binding.
fn collect_ignored_blocks(node: &ruby_prism::Node<'_>, ignored: &mut HashSet<usize>) {
    // CallNode: mark its block as ignored, recurse into receiver + arguments
    if let Some(call) = node.as_call_node() {
        if let Some(block) = call.block() {
            if let Some(block_node) = block.as_block_node() {
                ignored.insert(block_node.opening_loc().start_offset());
            }
        }
        if let Some(receiver) = call.receiver() {
            collect_ignored_blocks(&receiver, ignored);
        }
        if let Some(args) = call.arguments() {
            for arg in args.arguments().iter() {
                collect_ignored_blocks(&arg, ignored);
            }
        }
        return;
    }

    // KeywordHashNode (unbraced hash in argument position)
    if let Some(kwh) = node.as_keyword_hash_node() {
        for element in kwh.elements().iter() {
            collect_ignored_blocks(&element, ignored);
        }
        return;
    }

    // HashNode (braced hash) — skip per vendor logic (braces prevent rebinding)
    if node.as_hash_node().is_some() {
        return;
    }

    // AssocNode (key: value pair)
    if let Some(assoc) = node.as_assoc_node() {
        collect_ignored_blocks(&assoc.value(), ignored);
        return;
    }

    // AssocSplatNode (**hash)
    if let Some(splat) = node.as_assoc_splat_node() {
        if let Some(value) = splat.value() {
            collect_ignored_blocks(&value, ignored);
        }
        return;
    }

    // LambdaNode (`-> { ... }`) — in Parser AST, lambdas are block nodes.
    // RuboCop's `get_blocks` yields them, so `ignore_node` is called on the
    // lambda block. Any blocks nested inside the lambda body are then
    // suppressed by `part_of_ignored_node?`. We must recurse into the lambda's
    // body to find and ignore nested blocks.
    if let Some(lambda) = node.as_lambda_node() {
        if let Some(body) = lambda.body() {
            collect_ignored_blocks_from_body(&body, ignored);
        }
    }
}

/// Recursively find all blocks inside a node body and mark them as ignored.
/// Used for lambda bodies where we need to suppress all nested blocks.
fn collect_ignored_blocks_from_body(node: &ruby_prism::Node<'_>, ignored: &mut HashSet<usize>) {
    if let Some(call) = node.as_call_node() {
        if let Some(block) = call.block() {
            if let Some(block_node) = block.as_block_node() {
                ignored.insert(block_node.opening_loc().start_offset());
            }
        }
        if let Some(receiver) = call.receiver() {
            collect_ignored_blocks_from_body(&receiver, ignored);
        }
        if let Some(args) = call.arguments() {
            for arg in args.arguments().iter() {
                collect_ignored_blocks_from_body(&arg, ignored);
            }
        }
        if let Some(block) = call.block() {
            if let Some(block_node) = block.as_block_node() {
                if let Some(body) = block_node.body() {
                    collect_ignored_blocks_from_body(&body, ignored);
                }
            }
        }
        return;
    }

    if let Some(stmts) = node.as_statements_node() {
        for stmt in stmts.body().iter() {
            collect_ignored_blocks_from_body(&stmt, ignored);
        }
        return;
    }

    if let Some(hash) = node.as_hash_node() {
        for element in hash.elements().iter() {
            collect_ignored_blocks_from_body(&element, ignored);
        }
        return;
    }

    if let Some(hash) = node.as_keyword_hash_node() {
        for element in hash.elements().iter() {
            collect_ignored_blocks_from_body(&element, ignored);
        }
        return;
    }

    if let Some(assoc) = node.as_assoc_node() {
        collect_ignored_blocks_from_body(&assoc.key(), ignored);
        collect_ignored_blocks_from_body(&assoc.value(), ignored);
        return;
    }

    if let Some(splat) = node.as_assoc_splat_node() {
        if let Some(value) = splat.value() {
            collect_ignored_blocks_from_body(&value, ignored);
        }
        return;
    }

    if let Some(paren) = node.as_parentheses_node() {
        if let Some(body) = paren.body() {
            collect_ignored_blocks_from_body(&body, ignored);
        }
        return;
    }

    if let Some(array) = node.as_array_node() {
        for element in array.elements().iter() {
            collect_ignored_blocks_from_body(&element, ignored);
        }
        return;
    }

    if let Some(splat) = node.as_splat_node() {
        if let Some(expression) = splat.expression() {
            collect_ignored_blocks_from_body(&expression, ignored);
        }
        return;
    }

    if let Some(ret) = node.as_return_node() {
        if let Some(args) = ret.arguments() {
            for arg in args.arguments().iter() {
                collect_ignored_blocks_from_body(&arg, ignored);
            }
        }
        return;
    }

    if let Some(yield_node) = node.as_yield_node() {
        if let Some(args) = yield_node.arguments() {
            for arg in args.arguments().iter() {
                collect_ignored_blocks_from_body(&arg, ignored);
            }
        }
        return;
    }

    if let Some(lambda) = node.as_lambda_node() {
        if let Some(body) = lambda.body() {
            collect_ignored_blocks_from_body(&body, ignored);
        }
        return;
    }

    if let Some(interp) = node.as_interpolated_string_node() {
        for part in interp.parts().iter() {
            collect_ignored_blocks_from_body(&part, ignored);
        }
        return;
    }

    if let Some(embedded) = node.as_embedded_statements_node() {
        if let Some(stmts) = embedded.statements() {
            for stmt in stmts.body().iter() {
                collect_ignored_blocks_from_body(&stmt, ignored);
            }
        }
        return;
    }

    // Assignment nodes — recurse into the value expression
    // e.g., `result = items.find { |item| ... }` inside a lambda body
    if let Some(write) = node.as_local_variable_write_node() {
        collect_ignored_blocks_from_body(&write.value(), ignored);
        return;
    }
    if let Some(write) = node.as_instance_variable_write_node() {
        collect_ignored_blocks_from_body(&write.value(), ignored);
        return;
    }
    if let Some(write) = node.as_class_variable_write_node() {
        collect_ignored_blocks_from_body(&write.value(), ignored);
        return;
    }
    if let Some(write) = node.as_global_variable_write_node() {
        collect_ignored_blocks_from_body(&write.value(), ignored);
        return;
    }
    if let Some(write) = node.as_constant_write_node() {
        collect_ignored_blocks_from_body(&write.value(), ignored);
        return;
    }
    if let Some(write) = node.as_local_variable_operator_write_node() {
        collect_ignored_blocks_from_body(&write.value(), ignored);
        return;
    }
    if let Some(write) = node.as_instance_variable_operator_write_node() {
        collect_ignored_blocks_from_body(&write.value(), ignored);
        return;
    }
    // Multi-write: a, b = expr
    if let Some(write) = node.as_multi_write_node() {
        collect_ignored_blocks_from_body(&write.value(), ignored);
        return;
    }

    // IfNode, UnlessNode, etc. — recurse into their bodies for completeness
    if let Some(if_node) = node.as_if_node() {
        if let Some(stmts) = if_node.statements() {
            for stmt in stmts.body().iter() {
                collect_ignored_blocks_from_body(&stmt, ignored);
            }
        }
        if let Some(subsequent) = if_node.subsequent() {
            collect_ignored_blocks_from_body(&subsequent, ignored);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(BlockDelimiters, "cops/style/block_delimiters");
    crate::cop_variant_fixture_tests!(
        BlockDelimiters,
        "cops/style/block_delimiters",
        semantic,
        always_braces,
    );

    #[test]
    fn no_offense_proc_in_keyword_arg() {
        // Proc block in keyword arg without parens — changing braces would change semantics
        let source = b"my_method :arg1, arg2: proc {\n  something\n}, arg3: :another_value\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag proc block in keyword argument position, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_safe_navigation_non_parenthesized() {
        // Safe-navigation call with non-parenthesized block arg
        let source = b"foo&.bar baz {\n  y\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag block in safe-navigation non-parenthesized call, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_chained_method_block_in_arg() {
        // Block result chained and used as argument
        let source = b"foo bar + baz {\n}.qux.quux\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag chained block in non-parenthesized arg, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_lambda_in_keyword_arg_without_parens() {
        // lambda block in keyword arg of non-parenthesized call
        let source = b"foo :bar, :baz, qux: lambda { |a|\n  bar a\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag lambda block in keyword arg, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_nested_in_non_parens_arg() {
        // text html { body { ... } } — html's block is in non-parenthesized arg of text,
        // body's block is inside html's ignored block => both suppressed
        let source = b"text html {\n  body {\n    input(type: 'text')\n  }\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag blocks nested in non-parenthesized arg, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_deeply_nested_in_non_parens_arg() {
        // foo browser { text html { body { ... } } } — browser's block is in foo's
        // non-parens arg, all inner blocks are suppressed
        let source =
            b"foo browser {\n  text html {\n    body {\n      input(type: 'text')\n    }\n  }\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag deeply nested blocks in non-parens arg, got: {:?}",
            diags
        );
    }

    #[test]
    fn offense_only_outermost_nested_braces() {
        // When multiple multi-line brace blocks are nested, only the outermost
        // should be flagged (RuboCop's ignore_node behavior)
        let source = b"items.map {\n  items.select {\n    true\n  }\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag only outermost multi-line brace block, got: {:?}",
            diags
        );
        assert_eq!(diags[0].location.line, 1);
    }

    #[test]
    fn offense_only_outermost_in_chain() {
        // Chained blocks: a.select { ... }.reject { ... }.each { ... }
        // RuboCop flags only the outermost (last in chain) in Parser AST.
        // In Prism, the outermost CallNode covers the entire chain, so
        // suppressing via the call node's range suppresses inner blocks.
        let source = b"items.select {\n  x.valid?\n}.reject {\n  x.empty?\n}.each {\n  puts x\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag only the outermost chained block, got: {:?}",
            diags
        );
        // The outermost block in Prism is the top-level CallNode (.each)
        assert_eq!(diags[0].location.line, 5, "Should flag .each at line 5");
    }

    #[test]
    fn offense_two_block_chain() {
        // a.select { ... }.reject { ... } — only outermost flagged
        let source = b"items.select {\n  x.valid?\n}.reject {\n  x.empty?\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag only outermost in two-block chain, got: {:?}",
            diags
        );
        assert_eq!(diags[0].location.line, 3, "Should flag .reject at line 3");
    }

    #[test]
    fn offense_block_in_operator_arg() {
        // `a + b { ... }` — operator method with single block-bearing arg.
        // RuboCop does NOT ignore the block (single_argument_operator_method? skips
        // the ignore logic), so the multi-line brace block should be flagged.
        let source = b"a + b {\n  c\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag multi-line brace block in operator arg, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_do_end_single_line_rescue_array() {
        // Single-line do-end with rescue that has array exception type
        // This needs do-end because {} + rescue + array creates ambiguity
        let source = b"foo do next unless bar; rescue StandardError; end\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag single-line do-end with rescue+semicolon, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_block_in_string_concat_operator() {
        // `a + b.collect { ... }.join` — the `+` operator's argument is a send node
        // (not a block), so RuboCop does NOT skip ignore_node logic. The block is
        // found via get_blocks recursion and ignored.
        let source = b"result = prefix + items.collect { |i|\n  i.to_s\n}.join(\", \")\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag block inside operator argument chain, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_block_in_string_concat_multi_plus() {
        // Multiple `+` concatenation: `a + b.map { }.join + c`
        let source = b"x = \"prefix\" + items.map { |i|\n  i.to_s\n}.join(\", \") + \"suffix\"\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag block in multi-plus concat, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_block_as_rhs_of_or_assign_with_plus() {
        // `@x ||= a + b.collect { ... }.flatten` — the `+` operator's argument
        // is a send node (.flatten), so RuboCop ignores the inner block.
        let source = b"@x ||= prefix + items.collect { |m|\n  m.ancestors\n}.flatten\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag block in operator arg of ||= expression, got: {:?}",
            diags
        );
    }

    #[test]
    fn offense_super_multi_line_braces() {
        // `super(args) { ... }` — multi-line brace block on super should be flagged
        let source = b"super(num_waits) {\n  yield if block_given?\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag multi-line brace block on super, got: {:?}",
            diags
        );
    }

    #[test]
    fn offense_super_single_line_do_end() {
        // `super(*args) do |item| yielder << item end` — single-line do-end on super
        let source = b"super(*args) do |item| yielder << item end\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag single-line do-end block on super, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_super_multi_line_do_end() {
        // `super(args) do ... end` — correct style for multi-line
        let source = b"super(num_waits) do\n  yield if block_given?\nend\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag multi-line do-end block on super, got: {:?}",
            diags
        );
    }

    #[test]
    fn offense_forwarding_super_multi_line_braces() {
        // `super { ... }` with ForwardingSuperNode — multi-line braces should be flagged
        let source = b"super {\n  yield\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag multi-line brace block on bare super, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_block_in_parenthesized_arg() {
        // Block inside a parenthesized method call argument — parenthesized calls
        // don't trigger ignore_node, so block is checked normally.
        // In line_count_based, multi-line braces = offense.
        let source = b"foo(bar.map { |x|\n  x.to_s\n})\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag multi-line brace block in parenthesized arg, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_hash_bracket_with_block() {
        // Hash[list.map { ... }] — `[]` is a non-parenthesized method call
        let source = b"Hash[list.map { |k, v|\n  [k, v.to_s]\n}.sort_by(&:first)]\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag block inside Hash[] argument, got: {:?}",
            diags
        );
    }

    #[test]
    fn offense_multi_line_braces_when_chained() {
        let source = b"items.map { |x|\n  x.to_s\n}.join(\", \")\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag multi-line brace block even when chained, got: {:?}",
            diags
        );
    }

    #[test]
    fn offense_multi_line_braces_when_assigned() {
        let source = b"result = items.map { |x|\n  x.to_s\n}\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert_eq!(
            diags.len(),
            1,
            "Should flag multi-line brace block even when assigned, got: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_lambda_body_assignment_with_block() {
        // Block inside assignment inside lambda body in keyword arg
        let source = b"render node: -> {\n  result = items.find { |item|\n    item.name == \"test\"\n  }\n} do\n  puts \"rendered\"\nend\n";
        let diags = crate::testutil::run_cop_full(&BlockDelimiters, source);
        assert!(
            diags.is_empty(),
            "Should not flag block inside assignment in lambda body in keyword arg, got: {:?}",
            diags
        );
    }

    // --- Helper for creating config with EnforcedStyle ---

    fn config_with_style(style: &str) -> crate::cop::CopConfig {
        use std::collections::HashMap;
        let mut options: HashMap<String, serde_yml::Value> = HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String(style.to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    // =========== always_braces tests ===========

    #[test]
    fn always_braces_offense_multi_line_do_end() {
        let source = b"items.each do |x|\n  puts x\nend\n";
        let config = config_with_style("always_braces");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(
            diags[0]
                .message
                .contains("Prefer `{...}` over `do...end` for blocks.")
        );
    }

    #[test]
    fn always_braces_offense_single_line_do_end() {
        let source = b"each do |x| end\n";
        let config = config_with_style("always_braces");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(
            diags[0]
                .message
                .contains("Prefer `{...}` over `do...end` for blocks.")
        );
    }

    #[test]
    fn always_braces_no_offense_single_line_braces() {
        let source = b"each { |x| x }\n";
        let config = config_with_style("always_braces");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn always_braces_no_offense_multi_line_braces() {
        let source = b"each { |x|\n  x\n}\n";
        let config = config_with_style("always_braces");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn always_braces_no_offense_parser_incompatible_non_utf8_file() {
        let source = b"# encoding:windows-1252\nassert_match(/^(\\xdf)\\1$/i, \"\\xdf\\xdf\")\n[0x8a, 0x8c, 0x8e, *0xc0..0xd6, *0xd8..0xde, 0x9f].zip([0x9a, 0x9c, 0x9e, *0xe0..0xf6, *0xf8..0xfe, 0xff]).each do |c1, c2|\n  c1 = c1.chr(\"windows-1252\")\n  c2 = c2.chr(\"windows-1252\")\nend\n";
        let config = config_with_style("always_braces");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn always_braces_offense_builder_template_with_semantic_parse_error() {
        let source = b"xml.wrapper do\n  xml << yield\nend\n";
        let config = config_with_style("always_braces");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(
            diags[0]
                .message
                .contains("Prefer `{...}` over `do...end` for blocks.")
        );
    }

    #[test]
    fn always_braces_offense_non_utf8_file_with_plain_high_bytes() {
        let source = b"# encoding: iso-8859-1\nGiven(/^jeg drikker en \"([^\"]*)\"$/) do |drink|\n  expect(drink).to eq '\xF8l'.force_encoding(\"ISO-8859-1\").encode(\"UTF-8\")\nend\n";
        let config = config_with_style("always_braces");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(
            diags[0]
                .message
                .contains("Prefer `{...}` over `do...end` for blocks.")
        );
    }

    #[test]
    fn always_braces_no_offense_allowed_method() {
        let source = b"foo = lambda do\n  puts 42\nend\n";
        let config = config_with_style("always_braces");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn always_braces_offense_chained_do_end() {
        let source = b"each do |x|\nend.map(&:to_s)\n";
        let config = config_with_style("always_braces");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
    }

    // =========== braces_for_chaining tests ===========

    #[test]
    fn braces_for_chaining_offense_multi_line_chained_do_end() {
        let source = b"each do |x|\nend.map(&:to_s)\n";
        let config = config_with_style("braces_for_chaining");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(diags[0].message.contains("multi-line chained blocks"));
    }

    #[test]
    fn braces_for_chaining_no_offense_multi_line_chained_braces() {
        let source = b"each { |x|\n}.map(&:to_sym)\n";
        let config = config_with_style("braces_for_chaining");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn braces_for_chaining_offense_multi_line_braces_no_chain() {
        let source = b"each { |x|\n  x\n}\n";
        let config = config_with_style("braces_for_chaining");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(diags[0].message.contains("without chaining"));
    }

    #[test]
    fn braces_for_chaining_no_offense_multi_line_do_end_no_chain() {
        let source = b"each do |x|\n  x\nend\n";
        let config = config_with_style("braces_for_chaining");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn braces_for_chaining_offense_single_line_do_end() {
        let source = b"each do |x| end\n";
        let config = config_with_style("braces_for_chaining");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(diags[0].message.contains("single-line blocks"));
    }

    #[test]
    fn braces_for_chaining_no_offense_single_line_braces() {
        let source = b"each { |x| x }\n";
        let config = config_with_style("braces_for_chaining");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn braces_for_chaining_allows_braces_when_chained_via_bracket() {
        // `[{foo: :bar}].find { }.[:foo]` — [] is a chain
        let source = b"foo = [{foo: :bar}].find { |h|\n  h.key?(:foo)\n}[:foo]\n";
        let config = config_with_style("braces_for_chaining");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    // =========== semantic tests ===========

    #[test]
    fn semantic_offense_braces_procedural() {
        // Return value not used — procedural block should use do-end
        let source = b"each { |x|\n  x\n}\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(diags[0].message.contains("procedural blocks"));
    }

    #[test]
    fn semantic_offense_do_end_functional_assigned() {
        // Return value is assigned — functional block should use braces
        let source = b"foo = map do |x|\n  x\nend\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(diags[0].message.contains("functional blocks"));
    }

    #[test]
    fn semantic_offense_do_end_functional_attribute_assigned() {
        // foo.bar = map do ... end — attribute assignment
        let source = b"foo.bar = map do |x|\n  x\nend\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(diags[0].message.contains("functional blocks"));
    }

    #[test]
    fn semantic_no_offense_do_end_procedural() {
        // Return value not used — do-end is proper for procedural
        let source = b"each do |x|\n  puts x\nend\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_braces_functional_assigned() {
        // Return value is assigned — braces are proper for functional
        let source = b"foo = map { |x|\n  x\n}\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_braces_functional_chained() {
        // Return value is used via chaining — braces are proper
        let source = b"map { |x|\n  x\n}.inspect\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_braces_return_value_of_scope() {
        // Block is last expression in another block — return value of scope
        let source = b"block do\n  map { |x|\n    x\n  }\nend\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_do_end_return_value_of_scope() {
        // do-end block is last expression in scope — return_value_of_scope is true,
        // but do-end check only uses return_value_used?, not return_value_of_scope
        // Since rv_used is false, do-end is proper.
        let source = b"block do\n  map do |x|\n    x\n  end\nend\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_do_end_procedural_method() {
        // `tap` is a procedural method — do-end is always proper for procedural methods
        // even when return value is used
        let config = {
            use std::collections::HashMap;
            let mut options: HashMap<String, serde_yml::Value> = HashMap::new();
            options.insert(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("semantic".to_string()),
            );
            options.insert(
                "ProceduralMethods".to_string(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("tap".to_string())]),
            );
            crate::cop::CopConfig {
                options,
                ..crate::cop::CopConfig::default()
            }
        };
        let source = b"foo = bar.tap do |x|\n  x.age = 3\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_braces_functional_method() {
        // `let` is a functional method — braces are always proper
        let config = {
            use std::collections::HashMap;
            let mut options: HashMap<String, serde_yml::Value> = HashMap::new();
            options.insert(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("semantic".to_string()),
            );
            options.insert(
                "FunctionalMethods".to_string(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("let".to_string())]),
            );
            crate::cop::CopConfig {
                options,
                ..crate::cop::CopConfig::default()
            }
        };
        let source = b"let(:foo) {\n  x\n}\n";
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_braces_in_logical_or() {
        // Block used in logical or — rv_of_scope
        let source = b"any? { |c| c } || foo\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_braces_in_array() {
        // Block used in array element — rv_of_scope
        let source = b"[detect { true }, other]\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_braces_in_hash_value_arg() {
        // Block inside hash value argument: `foo(a: items.map { ... })`
        // RuboCop treats the pair value itself as rv_of_scope via the `:pair`
        // parent, not as rv_used through the outer method argument.
        let source = b"where(id: items.map { |x| x.id })\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_braces_in_if_condition() {
        // Block used as if condition — rv_used
        let source = b"if any? { |x| x }\n  return\nend\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_no_offense_braces_in_range() {
        // Block in range — rv_of_scope
        let source = b"detect { true }..other\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(diags.is_empty(), "got: {:?}", diags);
    }

    #[test]
    fn semantic_offense_do_end_in_parens_passed_to_method() {
        // `puts (map do |x| x end)` — return value used via parens
        let source = b"puts (map do |x|\n  x\nend)\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert_eq!(diags.len(), 1, "got: {:?}", diags);
        assert!(diags[0].message.contains("functional blocks"));
    }

    #[test]
    fn semantic_no_offense_do_end_chained_each_not_functional() {
        // `items.each do |x| end` — .each return value not used, should be procedural (do-end OK)
        // Regression: call_node_key must use message_loc, not location().start_offset(),
        // because chained calls share the same start_offset in Prism.
        let source = b"items.each do |x|\n  puts x\nend\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(
            diags.is_empty(),
            "standalone .each should not be flagged as functional: {:?}",
            diags
        );
    }

    #[test]
    fn semantic_no_offense_braces_in_compound_assignment() {
        // `command.extra_schemas += [...].map { |f| ... }` — rv used via +=
        let source =
            b"command.extra_schemas += [\"a\", \"b\"].\n  map { |f| File.expand_path(f) }\n";
        let config = config_with_style("semantic");
        let diags = crate::testutil::run_cop_full_with_config(&BlockDelimiters, source, config);
        assert!(
            diags.is_empty(),
            "braces in compound assignment should be functional: {:?}",
            diags
        );
    }
}
