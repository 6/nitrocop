use crate::cop::shared::node_type::DEF_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-11)
///
/// Corpus oracle reported FP=13, FN=0.
///
/// Logic fixes already applied:
/// - respect `minimum_target_ruby_version 3.0`
/// - skip setter methods (`def foo=(x)`)
/// - skip endless methods whose body is or contains a heredoc
///
/// Remaining FP root cause: RuboCop only handles instance-method `def` here; it does not
/// register `on_defs`. Prism represents singleton methods as `DefNode` with a receiver,
/// so nitrocop was incorrectly treating `def self.foo = ...` as eligible and flagging
/// multiline singleton endless methods in opal and ruby-next that RuboCop ignores.
///
/// Fix: return early for receiver-bearing `DefNode`s before applying the endless-method
/// style checks.
///
/// ## Variant-style divergence (2026-04-05)
///
/// Default `allow_single_line` behavior matched the corpus, but the non-default
/// `require_single_line` / `require_always` styles had large FN counts because
/// nitrocop only re-checked existing endless methods and never flagged regular
/// `def .. end` bodies that RuboCop converts to endless form.
///
/// RuboCop's `can_be_made_endless?` works on parser AST bodies, while Prism
/// wraps method bodies in a `StatementsNode`. The equivalent Prism condition is:
/// exactly one body statement, and that single statement is not an explicit
/// `begin .. end` body. The fix implements that mapping, preserves the existing
/// multiline endless-method checks, and honors `MaxLineLength` /
/// `LineLengthEnabled` when the endless replacement would be too long (including
/// `private def ...` / `protected def ...` prefixes on the same line).
///
/// ## Variant-style fixes (2026-04-06)
///
/// Five Prism-vs-parser-gem divergences fixed for `require_single_line` / `require_always`:
///
/// 1. **`too_long_when_made_endless`**: was using full `def_column` (including
///    indentation), but RuboCop only uses `modifier_offset` (distance from access
///    modifier like `private def`). Caused ~6000 FN for indented methods.
///
/// 2. **`can_be_made_endless` / ParenthesesNode**: in the parser gem, `(expr)` is
///    a `begin` type, so `can_be_made_endless?` rejects it. Prism uses
///    `ParenthesesNode`. Now excluded. Caused ~170 FP.
///
/// 3. **Operator method names**: `name.ends_with(b"=")` incorrectly skipped
///    operator methods like `==`, `!=`, `===`, `<=`, `>=`, `<=>`, `=~`. RuboCop's
///    `assignment_method?` excludes comparison operators.
///
/// 4. **`body_uses_heredoc` / InterpolatedStringNode**: RuboCop's `use_heredoc?`
///    only checks `:str` descendants (not `:dstr`), so interpolated heredocs
///    nested inside expressions (e.g. `super(<<~MSG) ... #{x}`) aren't detected.
///    Also, non-interpolated heredocs with multi-line content are `:dstr` in the
///    parser gem, so `each_descendant(:str)` misses them too.
///
/// 5. **`body_is_single_line` / BlockNode**: RuboCop overrides `single_line?` on
///    `BlockNode` to check only the block delimiters (`{`/`}`), not the full
///    expression. A multiline receiver with single-line block braces counts as
///    single-line for the body check.
pub struct EndlessMethod;

impl EndlessMethod {
    /// Returns true if the node has a heredoc opening (`<<`).
    fn has_heredoc_opening(opening: Option<ruby_prism::Location<'_>>) -> bool {
        opening.is_some_and(|loc| loc.as_slice().starts_with(b"<<"))
    }

    /// Returns true if the def node's body is or contains a heredoc.
    /// Mirrors RuboCop's `use_heredoc?`:
    ///   return true if body.any_str_type? && body.heredoc?
    ///   body.each_descendant(:str).any?(&:heredoc?)
    ///
    /// RuboCop first checks if the body itself is a string-type heredoc (catches
    /// both str and dstr). Then walks descendants for ONLY `:str` nodes (plain
    /// strings), NOT `:dstr` (interpolated strings). This means interpolated
    /// heredocs nested inside other expressions (e.g. `super(<<~MSG) ... #{x}`)
    /// are NOT detected — a RuboCop quirk we must replicate.
    fn body_uses_heredoc(def_node: &ruby_prism::DefNode<'_>) -> bool {
        use ruby_prism::Visit;

        let Some(body) = def_node.body() else {
            return false;
        };

        // First check: is the body's single statement itself a heredoc?
        // Matches RuboCop's `body.any_str_type? && body.heredoc?`
        // (any_str_type? includes both str and dstr)
        let is_direct_heredoc = if let Some(stmts) = body.as_statements_node() {
            let body_nodes: Vec<_> = stmts.body().iter().collect();
            body_nodes.len() == 1 && Self::is_string_heredoc(&body_nodes[0])
        } else {
            Self::is_string_heredoc(&body)
        };
        if is_direct_heredoc {
            return true;
        }

        // Second check: walk descendants for plain StringNode heredocs only.
        // Matches RuboCop's `body.each_descendant(:str).any?(&:heredoc?)`
        // Note: RuboCop only checks :str, NOT :dstr, so interpolated heredocs
        // nested inside other expressions are intentionally not detected.
        struct PlainStringHeredocVisitor {
            found: bool,
        }

        impl<'pr> Visit<'pr> for PlainStringHeredocVisitor {
            fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
                if !self.found
                    && node
                        .opening_loc()
                        .is_some_and(|loc| loc.as_slice().starts_with(b"<<"))
                {
                    // In the parser gem, a heredoc with multi-line content (2+
                    // newlines) becomes :dstr, not :str. RuboCop's
                    // `each_descendant(:str)` won't find :dstr nodes, so only
                    // single-line content heredocs (≤1 newline) are detected.
                    let content = node.unescaped();
                    let newline_count = content.iter().filter(|&&b| b == b'\n').count();
                    if newline_count <= 1 {
                        self.found = true;
                    }
                }
                if !self.found {
                    ruby_prism::visit_string_node(self, node);
                }
            }
        }

        let mut visitor = PlainStringHeredocVisitor { found: false };
        visitor.visit(&body);
        visitor.found
    }

    /// Check if a node is a string-type heredoc (str, dstr, xstr, dxstr).
    fn is_string_heredoc(node: &ruby_prism::Node<'_>) -> bool {
        if let Some(s) = node.as_string_node() {
            return Self::has_heredoc_opening(s.opening_loc());
        }
        if let Some(s) = node.as_interpolated_string_node() {
            return Self::has_heredoc_opening(s.opening_loc());
        }
        if let Some(s) = node.as_x_string_node() {
            return Self::has_heredoc_opening(Some(s.opening_loc()));
        }
        if let Some(s) = node.as_interpolated_x_string_node() {
            return Self::has_heredoc_opening(Some(s.opening_loc()));
        }
        false
    }

    fn is_single_line(source: &SourceFile, loc: &ruby_prism::Location<'_>) -> bool {
        let (start_line, _) = source.offset_to_line_col(loc.start_offset());
        let (end_line, _) = source.offset_to_line_col(loc.end_offset());
        start_line == end_line
    }

    /// Checks if the method body is "single line" matching RuboCop's semantics.
    /// RuboCop overrides `single_line?` on BlockNode to check only the block
    /// delimiters (`{`/`}` or `do`/`end`), not the full expression. In the parser
    /// gem the body IS the block node; in Prism the body is a StatementsNode
    /// containing a CallNode that has a block. We replicate the override here.
    fn body_is_single_line(source: &SourceFile, body: &ruby_prism::Node<'_>) -> bool {
        // Unwrap StatementsNode to get the single statement
        let stmt = if let Some(stmts) = body.as_statements_node() {
            let nodes: Vec<_> = stmts.body().iter().collect();
            if nodes.len() == 1 {
                Some(nodes.into_iter().next().unwrap())
            } else {
                return false;
            }
        } else {
            None
        };

        let node = stmt.as_ref().unwrap_or(body);

        // If the node is a CallNode with a block, use block delimiter lines
        // (replicates RuboCop's BlockNode#single_line? override)
        if let Some(call) = node.as_call_node() {
            if let Some(block) = call.block() {
                if let Some(block_node) = block.as_block_node() {
                    let open_line = source
                        .offset_to_line_col(block_node.opening_loc().start_offset())
                        .0;
                    let close_line = source
                        .offset_to_line_col(block_node.closing_loc().start_offset())
                        .0;
                    return open_line == close_line;
                }
            }
        }

        // Default: check full expression span
        Self::is_single_line(source, &node.location())
    }

    fn single_body_statement(body: ruby_prism::Node<'_>) -> Option<ruby_prism::Node<'_>> {
        if let Some(stmts) = body.as_statements_node() {
            let body_nodes: Vec<_> = stmts.body().iter().collect();
            if body_nodes.len() == 1 {
                body_nodes.into_iter().next()
            } else {
                None
            }
        } else {
            Some(body)
        }
    }

    fn can_be_made_endless(def_node: &ruby_prism::DefNode<'_>) -> bool {
        let Some(body) = def_node.body() else {
            return false;
        };
        let Some(stmt) = Self::single_body_statement(body) else {
            return false;
        };
        // Prism: BeginNode = explicit `begin..end` keyword
        // Prism: ParenthesesNode = `(expr)` — in the parser gem this is `begin_type?`,
        // so RuboCop's `can_be_made_endless?` rejects it.
        stmt.as_begin_node().is_none() && stmt.as_parentheses_node().is_none()
    }

    fn arguments_source(source: &SourceFile, def_node: &ruby_prism::DefNode<'_>) -> String {
        let Some(params) = def_node.parameters() else {
            return String::new();
        };

        if let (Some(lparen), Some(rparen)) = (def_node.lparen_loc(), def_node.rparen_loc()) {
            return source
                .byte_slice(lparen.start_offset(), rparen.end_offset(), "")
                .to_string();
        }

        let params_loc = params.location();
        let params_src = source.byte_slice(params_loc.start_offset(), params_loc.end_offset(), "");
        if params_src.is_empty() {
            String::new()
        } else {
            format!(" {params_src}")
        }
    }

    fn endless_replacement_length(
        source: &SourceFile,
        def_node: &ruby_prism::DefNode<'_>,
    ) -> usize {
        let body = match def_node.body() {
            Some(body) => body,
            None => return 0,
        };
        let body_loc = body.location();
        let body_src = source.byte_slice(body_loc.start_offset(), body_loc.end_offset(), "");
        let method_name = std::str::from_utf8(def_node.name().as_slice()).unwrap_or("");
        let arguments = Self::arguments_source(source, def_node);

        "def ".len() + method_name.len() + arguments.len() + " = ".len() + body_src.len()
    }

    /// Compute the modifier offset, matching RuboCop's `modifier_offset(node)`.
    /// RuboCop returns `node.loc.column - node.parent.loc.column` when the parent
    /// is on the same line (i.e. `private def foo`), otherwise 0.
    /// We approximate this by checking if there is non-whitespace text before the
    /// `def` keyword on its line — that text is the access modifier.
    fn modifier_offset(source: &SourceFile, def_node: &ruby_prism::DefNode<'_>) -> usize {
        let def_start = def_node.def_keyword_loc().start_offset();
        let (_, def_column) = source.offset_to_line_col(def_start);
        if def_column == 0 {
            return 0;
        }
        // Walk back from the def keyword to the start of the line
        let line_start_offset = def_start - def_column;
        let prefix = source.byte_slice(line_start_offset, def_start, "");
        // If the prefix before `def` is all whitespace, there's no modifier
        let first_non_ws = prefix.find(|c: char| !c.is_ascii_whitespace());
        match first_non_ws {
            Some(pos) => def_column - pos,
            None => 0,
        }
    }

    fn too_long_when_made_endless(
        source: &SourceFile,
        def_node: &ruby_prism::DefNode<'_>,
        config: &CopConfig,
    ) -> bool {
        if !config.get_bool("LineLengthEnabled", true) {
            return false;
        }

        let max_line_length = config.get_usize("MaxLineLength", 120);
        // RuboCop only accounts for the access modifier prefix (e.g., `private `)
        // when computing line length, NOT the full indentation.
        let offset = Self::modifier_offset(source, def_node);
        offset + Self::endless_replacement_length(source, def_node) > max_line_length
    }
}

impl Cop for EndlessMethod {
    fn name(&self) -> &'static str {
        "Style/EndlessMethod"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[DEF_NODE]
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
        // RuboCop: minimum_target_ruby_version 3.0
        let ruby_version = config
            .options
            .get("TargetRubyVersion")
            .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
            .unwrap_or(2.7);
        if ruby_version < 3.0 {
            return;
        }

        let def_node = match node.as_def_node() {
            Some(d) => d,
            None => return,
        };

        // RuboCop implements only `on_def`, not `on_defs`, for this cop.
        // Prism represents singleton methods as DefNode with a receiver.
        if def_node.receiver().is_some() {
            return;
        }

        // RuboCop: return if node.assignment_method?
        // Skip setter/assignment methods (e.g. `def foo=(x)`, `def []=(k,v)`) — they
        // end with '=' but are NOT comparison operators. RuboCop's comparison_method?
        // set is: ==, ===, !=, <=>, >=, <=, =~
        let name = def_node.name();
        let name_bytes = name.as_slice();
        if name_bytes.ends_with(b"=")
            && !matches!(
                name_bytes,
                b"==" | b"===" | b"!=" | b"<=>" | b">=" | b"<=" | b"=~"
            )
        {
            return;
        }

        // RuboCop: return if use_heredoc?(node)
        // Skip methods whose body is or contains a heredoc.
        // Heredocs in Prism are StringNode/InterpolatedStringNode with opening starting with "<<".
        if Self::body_uses_heredoc(&def_node) {
            return;
        }

        let style = config.get_str("EnforcedStyle", "allow_single_line");
        let is_endless = def_node.end_keyword_loc().is_none() && def_node.equal_loc().is_some();

        match style {
            "disallow" => {
                if is_endless {
                    let loc = def_node.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Avoid endless method definitions.".to_string(),
                    ));
                }
            }
            "allow_single_line" => {
                if is_endless {
                    let loc = def_node.location();
                    if !Self::is_single_line(source, &loc) {
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Avoid endless method definitions with multiple lines.".to_string(),
                        ));
                    }
                }
            }
            "allow_always" => {
                // No offenses for endless methods
            }
            "require_single_line" => {
                if is_endless {
                    let loc = def_node.location();
                    if !Self::is_single_line(source, &loc) {
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Avoid endless method definitions with multiple lines.".to_string(),
                        ));
                    }
                } else if Self::can_be_made_endless(&def_node)
                    && def_node
                        .body()
                        .is_some_and(|body| Self::body_is_single_line(source, &body))
                    && !Self::too_long_when_made_endless(source, &def_node, config)
                {
                    let loc = def_node.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use endless method definitions for single line methods.".to_string(),
                    ));
                }
            }
            "require_always" => {
                if !is_endless
                    && Self::can_be_made_endless(&def_node)
                    && !Self::too_long_when_made_endless(source, &def_node, config)
                {
                    let loc = def_node.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use endless method definitions.".to_string(),
                    ));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::CopConfig;
    use crate::testutil::run_cop_full_with_config;

    fn ruby30_config() -> CopConfig {
        let mut config = CopConfig::default();
        config.options.insert(
            "TargetRubyVersion".to_string(),
            serde_yml::Value::Number(serde_yml::Number::from(3.0)),
        );
        config
    }

    fn ruby30_style_config(style: &str) -> CopConfig {
        let mut config = ruby30_config();
        config.options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String(style.to_string()),
        );
        config
    }

    fn ruby30_style_with_line_length(style: &str, max: u64, enabled: bool) -> CopConfig {
        let mut config = ruby30_style_config(style);
        config.options.insert(
            "MaxLineLength".to_string(),
            serde_yml::Value::Number(serde_yml::Number::from(max)),
        );
        config.options.insert(
            "LineLengthEnabled".to_string(),
            serde_yml::Value::Bool(enabled),
        );
        config
    }

    #[test]
    fn offense_with_ruby30() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &EndlessMethod,
            include_bytes!("../../../tests/fixtures/cops/style/endless_method/offense.rb"),
            ruby30_config(),
        );
    }

    #[test]
    fn no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &EndlessMethod,
            include_bytes!("../../../tests/fixtures/cops/style/endless_method/no_offense.rb"),
            ruby30_config(),
        );
    }

    #[test]
    fn require_single_line_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &EndlessMethod,
            include_bytes!(
                "../../../tests/fixtures/cops/style/endless_method/require_single_line_offense.rb"
            ),
            ruby30_style_config("require_single_line"),
        );
    }

    #[test]
    fn require_single_line_no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &EndlessMethod,
            include_bytes!(
                "../../../tests/fixtures/cops/style/endless_method/require_single_line_no_offense.rb"
            ),
            ruby30_style_config("require_single_line"),
        );
    }

    #[test]
    fn require_always_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &EndlessMethod,
            include_bytes!(
                "../../../tests/fixtures/cops/style/endless_method/require_always_offense.rb"
            ),
            ruby30_style_config("require_always"),
        );
    }

    #[test]
    fn require_always_no_offense() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &EndlessMethod,
            include_bytes!(
                "../../../tests/fixtures/cops/style/endless_method/require_always_no_offense.rb"
            ),
            ruby30_style_config("require_always"),
        );
    }

    #[test]
    fn require_single_line_respects_line_length() {
        let source =
            b"def my_method\n  'this_string_ends_at_column_75_________________________________________'\nend\n";
        let diags = run_cop_full_with_config(
            &EndlessMethod,
            source,
            ruby30_style_with_line_length("require_single_line", 80, true),
        );
        assert!(
            diags.is_empty(),
            "Endless replacement exceeding MaxLineLength should be skipped, got: {diags:?}"
        );
    }

    #[test]
    fn require_single_line_ignores_line_length_when_disabled() {
        let source =
            b"def my_method\n  'this_string_ends_at_column_75_________________________________________'\nend\n";
        let diags = run_cop_full_with_config(
            &EndlessMethod,
            source,
            ruby30_style_with_line_length("require_single_line", 80, false),
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].message,
            "Use endless method definitions for single line methods."
        );
    }

    #[test]
    fn require_single_line_flags_access_modifier_def() {
        let source = b"private def my_method\n  x\nend\n";
        let diags = run_cop_full_with_config(
            &EndlessMethod,
            source,
            ruby30_style_config("require_single_line"),
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 8);
        assert_eq!(
            diags[0].message,
            "Use endless method definitions for single line methods."
        );
    }

    #[test]
    fn require_single_line_skips_regular_heredoc_body() {
        let source = b"def my_method\n  <<~HEREDOC\n    hello\n  HEREDOC\nend\n";
        let diags = run_cop_full_with_config(
            &EndlessMethod,
            source,
            ruby30_style_config("require_single_line"),
        );
        assert!(
            diags.is_empty(),
            "Heredoc bodies should be skipped, got: {diags:?}"
        );
    }

    #[test]
    fn require_single_line_indentation_not_counted_in_line_length() {
        // RuboCop's `too_long_when_made_endless?` does NOT count indentation —
        // only the replacement text length + modifier offset. `def x = y` is 9
        // chars, under limit 80, so it should be flagged even at deep indentation.
        let mut source = Vec::new();
        source.extend_from_slice(&[b' '; 76]);
        source.extend_from_slice(b"def x\n  y\nend\n");
        let diags = run_cop_full_with_config(
            &EndlessMethod,
            &source,
            ruby30_style_with_line_length("require_single_line", 80, true),
        );
        assert_eq!(
            diags.len(),
            1,
            "Indentation should NOT be counted in line length check (matching RuboCop), got: {diags:?}"
        );
    }

    #[test]
    fn require_single_line_modifier_offset_counted_in_line_length() {
        // `private def my_method = x` is 25 chars. The modifier offset is 8
        // (len("private ")), so total = 25 + 8... wait, no. The replacement is
        // "def my_method = x" (17 chars) and modifier_offset is 8. Total = 25.
        // With Max 24, this should be too long and skipped.
        let source = b"private def my_method\n  x\nend\n";
        let diags = run_cop_full_with_config(
            &EndlessMethod,
            source,
            ruby30_style_with_line_length("require_single_line", 24, true),
        );
        assert!(
            diags.is_empty(),
            "Modifier offset should be counted in line length check, got: {diags:?}"
        );
    }

    #[test]
    fn require_always_skips_regular_xstring_heredoc_body() {
        let source = b"def my_method\n  <<~`HEREDOC`\n    echo hello\n  HEREDOC\nend\n";
        let diags = run_cop_full_with_config(
            &EndlessMethod,
            source,
            ruby30_style_config("require_always"),
        );
        assert!(
            diags.is_empty(),
            "XString heredoc bodies should be skipped, got: {diags:?}"
        );
    }
}
