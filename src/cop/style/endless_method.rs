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
pub struct EndlessMethod;

impl EndlessMethod {
    /// Returns true if the def node's body is or contains a heredoc.
    /// Mirrors RuboCop's `use_heredoc?` by walking string/xstring nodes under
    /// the body and checking whether their opening delimiter starts with `<<`.
    fn body_uses_heredoc(def_node: &ruby_prism::DefNode<'_>) -> bool {
        use ruby_prism::Visit;

        struct HeredocVisitor {
            found: bool,
        }

        impl HeredocVisitor {
            fn visit_heredoc<'pr>(&mut self, opening: Option<ruby_prism::Location<'pr>>) {
                if self.found {
                    return;
                }
                if opening.is_some_and(|loc| loc.as_slice().starts_with(b"<<")) {
                    self.found = true;
                }
            }
        }

        impl<'pr> Visit<'pr> for HeredocVisitor {
            fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
                self.visit_heredoc(node.opening_loc());
                if !self.found {
                    ruby_prism::visit_string_node(self, node);
                }
            }

            fn visit_interpolated_string_node(
                &mut self,
                node: &ruby_prism::InterpolatedStringNode<'pr>,
            ) {
                self.visit_heredoc(node.opening_loc());
                if !self.found {
                    ruby_prism::visit_interpolated_string_node(self, node);
                }
            }

            fn visit_x_string_node(&mut self, node: &ruby_prism::XStringNode<'pr>) {
                self.visit_heredoc(Some(node.opening_loc()));
                if !self.found {
                    ruby_prism::visit_x_string_node(self, node);
                }
            }

            fn visit_interpolated_x_string_node(
                &mut self,
                node: &ruby_prism::InterpolatedXStringNode<'pr>,
            ) {
                self.visit_heredoc(Some(node.opening_loc()));
                if !self.found {
                    ruby_prism::visit_interpolated_x_string_node(self, node);
                }
            }
        }

        let Some(body) = def_node.body() else {
            return false;
        };

        let mut visitor = HeredocVisitor { found: false };
        visitor.visit(&body);
        visitor.found
    }

    fn is_single_line(source: &SourceFile, loc: &ruby_prism::Location<'_>) -> bool {
        let (start_line, _) = source.offset_to_line_col(loc.start_offset());
        let (end_line, _) = source.offset_to_line_col(loc.end_offset());
        start_line == end_line
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
        stmt.as_begin_node().is_none()
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

        "def ".chars().count()
            + method_name.chars().count()
            + arguments.chars().count()
            + " = ".chars().count()
            + body_src.chars().count()
    }

    fn modifier_offset(source: &SourceFile, def_node: &ruby_prism::DefNode<'_>) -> usize {
        let def_loc = def_node.def_keyword_loc();
        let (line, _) = source.offset_to_line_col(def_loc.start_offset());
        let line_start = source.line_start_offset(line);
        let prefix = source.byte_slice(line_start, def_loc.start_offset(), "");
        let trimmed = prefix.trim_start_matches(char::is_whitespace);
        if trimmed.is_empty() {
            0
        } else {
            trimmed.chars().count()
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
        Self::endless_replacement_length(source, def_node) + Self::modifier_offset(source, def_node)
            > max_line_length
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
        // Skip setter methods (e.g. def foo=(x)) — they end with '='
        let name = def_node.name();
        let name_bytes = name.as_slice();
        if name_bytes.ends_with(b"=") {
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
                        .is_some_and(|body| Self::is_single_line(source, &body.location()))
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
